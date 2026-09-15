# Admin Gestion Photocopie — tableau de bord du porteur du projet

Petite appli web (Cloudflare Workers + D1), accessible depuis un téléphone
ou un PC avec internet — pas depuis le PC d'une boutique, qui n'est jamais
connecté. Sert à :

- garder la liste des boutiques déployées (nom, gérant, téléphone,
  identifiant machine) ;
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
est déjà créée et migrée en prod (5 tables : `boutiques`, `licences`,
`rapports`, `demandes`, `parametres`). `wrangler.toml` pointe déjà dessus.

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
     | `LICENSE_SECRET` | **copie exacte** de la constante `SECRET` dans `src-tauri/src/license.rs` — voir l'avertissement ci-dessous |

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

## Secret de licence — le point le plus important

`LICENSE_SECRET` doit être **exactement identique** à la constante `SECRET`
dans `src-tauri/src/license.rs` — sinon les clés générées ici seraient
rejetées par l'application des gérants (et vice-versa). Si tu changes un
jour le secret dans `license.rs`, remets aussi à jour le secret GitHub
`LICENSE_SECRET` (même page que l'étape 2 ci-dessus, "Update"), sinon les
deux ne seront plus synchronisés.

## Tester en local avant de déployer (facultatif, réservé au développement)

```bash
npm run db:migrate:local       # recrée les tables dans une copie locale de D1
npm run dev                    # wrangler dev, http://localhost:8787
```

En local, `wrangler dev` te demande aussi `ADMIN_PASSWORD`/`LICENSE_SECRET`
(via un fichier `.dev.vars` à créer toi-même, jamais commité — voir la doc
Wrangler) plutôt que les vrais secrets de prod.

## Après le premier déploiement

Va dans **Paramètres** (lien en haut de la page d'accueil) pour renseigner
tes numéros Mobile Money, un montant indicatif et ton contact WhatsApp —
c'est ce que voient les gérants sur `/renouveler`. Tant que ce n'est pas
rempli, la page de renouvellement s'affiche sans ces informations.

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
