# Admin Gestion Photocopie — tableau de bord du porteur du projet

Petite appli web (Cloudflare Workers + D1), accessible depuis un téléphone
ou un PC avec internet — pas depuis le PC d'une boutique, qui n'est jamais
connecté. Sert à :

- garder la liste des boutiques déployées (nom, gérant, téléphone,
  identifiant machine) ;
- générer les clés de licence (remplace `generer-licence.exe` — même
  algorithme, même résultat, vérifié) avec un historique automatique,
  sans rien à noter à la main ;
- voir d'un coup d'œil les abonnements qui arrivent bientôt à expiration.

La base D1 `photocopie-admin-db` (id `ca1a0e15-7e38-4eee-afb6-a44b5f6b4418`)
est déjà créée et migrée en prod (3 tables : `boutiques`, `licences`,
`rapports`). `wrangler.toml` pointe déjà dessus. Reste à déployer le
Worker lui-même, ce que le connecteur Cloudflare utilisé pendant le
développement ne permet pas de faire (il peut lire/lister les Workers,
pas en déployer) — à faire depuis ta machine :

```bash
npm install -g wrangler        # si pas déjà installé
cd admin
npm install
wrangler login                 # ouvre le navigateur, connecte le même compte Cloudflare
wrangler secret put ADMIN_PASSWORD    # le mot de passe pour toi-même, choisis-en un fort
wrangler secret put LICENSE_SECRET    # voir "Secret de licence" ci-dessous — TRÈS IMPORTANT
npm run deploy                 # déploie le Worker
```

## Secret de licence — le point le plus important

`LICENSE_SECRET` doit être **exactement identique** à la constante `SECRET`
dans `src-tauri/src/license.rs` — sinon les clés générées ici seraient
rejetées par l'application des gérants (et vice-versa). Ouvre ce fichier,
copie la valeur telle quelle, colle-la quand `wrangler secret put` te la
demande. Si tu changes un jour le secret dans `license.rs` (par exemple en
recompilant l'appli avec un nouveau secret), il faut aussi le remettre à
jour ici avec `wrangler secret put LICENSE_SECRET`, sinon les deux ne
seront plus synchronisés.

## Tester en local avant de déployer

```bash
npm run db:migrate:local       # recrée les tables dans une copie locale de D1
npm run dev                    # wrangler dev, http://localhost:8787
```

En local, `wrangler dev` te demande aussi `ADMIN_PASSWORD`/`LICENSE_SECRET`
(via un fichier `.dev.vars` à créer toi-même, jamais commité — voir la doc
Wrangler) plutôt que les vrais secrets de prod.

## Ce qui n'est pas encore fait

- Import des "rapports de visite" (table `rapports` déjà créée, mais pas
  encore de bouton "Exporter" côté appli du gérant, ni de page d'import
  ici).
- Page de téléchargement publique de l'appli (aujourd'hui, le `.exe` est
  uniquement récupérable via les artefacts GitHub Actions).
- Écran technique caché dans l'appli du gérant pour le dépannage sur
  place.
