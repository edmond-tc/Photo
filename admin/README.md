# Admin Gestion Photocopie — tableau de bord du porteur du projet

Petite appli web (Cloudflare Workers + D1), accessible depuis un téléphone
ou un PC avec internet — pas depuis le PC d'une boutique, qui n'est jamais
connecté. Sert à :

- garder la liste des boutiques déployées (nom, gérant, téléphone,
  identifiant machine) ;
- valider chaque installation AVANT qu'elle n'ait lieu, via un code signé
  que toi seul peux générer (page `/installations`) — indispensable dès
  qu'une installation est faite par quelqu'un d'autre que toi (agent,
  maintenancier) : sans ce code, le logiciel refuse de démarrer, même en
  essai gratuit, donc tu es forcément mis au courant ;
- générer les clés de licence (remplace `generer-licence.exe` — même
  algorithme, même résultat, vérifié) avec un historique automatique,
  sans rien à noter à la main ;
- voir d'un coup d'œil les abonnements qui arrivent bientôt à expiration ;
- laisser un gérant demander lui-même son renouvellement (page publique
  `/renouveler`, sans compte ni mot de passe) : il paie au numéro Mobile
  Money affiché, indique son paiement dans un formulaire, et la demande
  apparaît sur ta page d'accueil. Tu vérifies (le porteur du projet n'a
  pas de compte marchand API, donc pas de confirmation automatique du
  paiement) et cliques "Confirmer" — la clé se génère à ce moment-là,
  la boutique est créée automatiquement si c'est un nouveau gérant.

La base D1 `photocopie-admin-db` (id `ca1a0e15-7e38-4eee-afb6-a44b5f6b4418`)
est déjà créée et migrée en prod (6 tables : `boutiques`, `licences`,
`rapports`, `demandes`, `parametres`, `codes_installation`). `wrangler.toml`
pointe déjà dessus. Chaque déploiement (workflow `Deploy Admin`) rejoue tous
les fichiers de `migrations/` : rien à faire à la main pour une base déjà en
place, une nouvelle table apparaît toute seule au prochain déploiement.

## Déploiement — sans terminal, GitHub s'en charge

Un workflow (`.github/workflows/deploy-admin.yml`) déploie automatiquement
le Worker à chaque changement poussé dans `admin/` — exactement comme la
compilation automatique du `.exe` Windows. **Aucune commande à taper** :
juste remplir 4 "secrets" une seule fois, dans deux pages web.

**1. Créer un jeton d'accès Cloudflare** (page web, pas de terminal) :
   - Va sur [dash.cloudflare.com/profile/api-tokens](https://dash.cloudflare.com/profile/api-tokens)
   - "Create Token" → modèle "Edit Cloudflare Workers" → suivre les
     étapes → "Continue to summary" → "Create Token"
   - Copie le jeton affiché (il ne sera plus jamais montré après)
   - Sur la même page Cloudflare, en bas à droite du tableau de bord
     "Workers & Pages", note aussi l'**Account ID** affiché

**2. Ajouter 4 secrets sur GitHub** (page web, pas de terminal) :
   - Sur la page du dépôt GitHub → **Settings** → **Secrets and variables**
     → **Actions** → **New repository secret**, répéter 4 fois :

     | Nom du secret | Valeur |
     |---|---|
     | `CLOUDFLARE_API_TOKEN` | le jeton copié à l'étape 1 |
     | `CLOUDFLARE_ACCOUNT_ID` | l'Account ID noté à l'étape 1 |
     | `ADMIN_PASSWORD` | le mot de passe que tu veux utiliser pour te connecter au tableau de bord |
     | `CLE_PRIVEE_LICENCE` | la clé privée de signature (64 caractères) — voir ci-dessous |

   Note : `ADMIN_PASSWORD` et `CLE_PRIVEE_LICENCE` se définissent **côté
   Cloudflare**, pas côté GitHub (`npx wrangler secret put ADMIN_PASSWORD`,
   ou Cloudflare → Workers → photocopie-admin → Settings → Variables). Seuls
   `CLOUDFLARE_API_TOKEN` et `CLOUDFLARE_ACCOUNT_ID` sont des secrets GitHub :
   le déploiement automatique ne fonctionne pas tant qu'ils manquent.

**3.** Onglet **Actions** du dépôt → workflow **Deploy Admin** → **Run workflow**
   (ou attendre le prochain push touchant `admin/`, qui le déclenche tout seul).

Une fois terminé (quelques dizaines de secondes), le tableau de bord est en
ligne à l'adresse affichée dans les logs du workflow (`https://photocopie-admin.<ton-sous-domaine>.workers.dev`).

**4. Déposer l'installateur pour la page de téléchargement publique**
   (même visite, un pas de plus — page web, toujours pas de terminal) :
   - Récupère le `.exe` le plus récent : onglet **Actions** du dépôt →
     dernier run de *Build Windows* → artifact `photocopie-benin-windows`
     → télécharge et dézippe, tu obtiens un fichier `.exe` dans le dossier
     `bundle/nsis/`.
   - Sur Cloudflare : menu ☰ → **R2** → bucket **photocopie-telechargements**
     → **Upload** → choisis ce fichier.
   - **Important** : une fois uploadé, renomme-le (bouton "..." à côté du
     fichier → Rename, ou re-upload avec le bon nom) pour qu'il s'appelle
     **exactement** `GestionPhotocopie-Installateur.exe` — le Worker ne
     sert que ce nom précis.
   - La page `https://<ton-worker>.workers.dev/telecharger` devient alors
     utilisable : n'importe qui peut y télécharger l'appli sans compte
     GitHub, à partager largement (affiche, réseaux sociaux, WhatsApp...).
   - À refaire (juste le re-upload, pas le reste) à chaque nouvelle version
     que tu veux distribuer publiquement.

## Clé de licence — le point le plus important

Les licences sont signées avec une **paire de clés** : une clé privée qui
signe, une clé publique qui vérifie.

- La **clé privée** (`CLE_PRIVEE_LICENCE`, 64 caractères) ne doit exister
  qu'à deux endroits : dans les secrets Cloudflare de ce Worker, et dans ta
  propre sauvegarde personnelle (clé USB, gestionnaire de mots de passe).
  **Jamais** dans le dépôt, jamais sur le PC d'un gérant, jamais dans une
  conversation. Qui la possède peut fabriquer des licences à vie.
- La **clé publique** est écrite en clair dans `src-tauri/src/license.rs`
  (constante `CLE_PUBLIQUE`). C'est normal et sans danger : elle ne sait que
  vérifier une clé, jamais en créer.

Les deux vont ensemble. Si tu regénères une paire, il faut mettre à jour les
deux côtés **et** recompiler l'application, sinon les clés générées ici
seront rejetées par les gérants.

### Changer de paire de clés (en cas de fuite)

1. Génère une nouvelle paire.
2. Remplace `CLE_PUBLIQUE` dans `src-tauri/src/license.rs`, pousse, attends
   la compilation, redistribue l'installateur.
3. Mets à jour le secret `CLE_PRIVEE_LICENCE` côté Cloudflare.
4. Regénère une clé pour chaque boutique active depuis le tableau de bord.

## Tester en local avant de déployer (facultatif, réservé au développement)

```bash
npm run db:migrate:local       # recrée les tables dans une copie locale de D1
npm run dev                    # wrangler dev, http://localhost:8787
```

En local, `wrangler dev` te demande aussi `ADMIN_PASSWORD`/`CLE_PRIVEE_LICENCE`
(via un fichier `.dev.vars` à créer toi-même, jamais commité — voir la doc
Wrangler) plutôt que les vrais secrets de prod.

## Après le premier déploiement

Va dans **Paramètres** (lien en haut de la page d'accueil) pour renseigner
tes numéros Mobile Money, un montant indicatif et ton contact WhatsApp —
c'est ce que voient les gérants sur `/renouveler`. Tant que ce n'est pas
rempli, la page de renouvellement s'affiche sans ces informations.

## Code d'installation — savoir chaque installation faite

Distinct de la licence : une licence dit "cette machine a payé jusqu'à telle
date", un code d'installation dit juste "le porteur du projet a été prévenu
AVANT que cette machine précise soit installée". Utile dès qu'une
installation est faite par quelqu'un d'autre que toi (agent, maintenancier
sur le terrain) : sans code valide, le logiciel refuse de démarrer chez le
gérant — même en essai gratuit de 30 jours — donc chaque installation passe
forcément par toi, avant même de savoir si elle deviendra payante.

Signé avec la même paire de clés Ed25519 que les licences (même
`CLE_PRIVEE_LICENCE`), mais un message et un préfixe différents ("INSTALL|"
côté signature, "INST-" au début du code) : un code d'installation ne peut
donc jamais être confondu avec — ni recyclé comme — une clé de licence, dans
un sens ou dans l'autre.

Utilisation : page **Installations** (lien en haut de la page d'accueil) →
coller l'identifiant machine dicté par la personne sur place, une note
optionnelle (qui installe, où), "Générer le code" → le code s'affiche et
part par WhatsApp. Chaque code généré reste dans l'historique de cette page,
consultable à tout moment.

## Rapports de visite

Dans l'appli du gérant : **Réglages → Rapport pour le porteur du projet →
Générer le rapport** produit un petit texte (JSON) résumant l'état de
cette boutique (version, licence, dernière sauvegarde, dernière activité).
Récupère-le (clé USB, ou WhatsApp si le gérant a du réseau) et colle-le
dans la fiche de la boutique ici → "+ Importer un nouveau rapport". Ça
construit un historique dans le temps, sans jamais exiger que le PC de la
boutique soit connecté.

## Ce qui n'est pas encore fait

Rien de bloquant côté code — tout est construit et testé. Reste seulement
le déploiement initial (étapes 1 à 4 ci-dessus), qui demande un accès
direct au compte Cloudflare/GitHub.
