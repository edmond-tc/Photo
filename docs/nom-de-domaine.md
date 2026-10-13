# Nom de domaine : comment passer de « workers.dev » à votre propre adresse

Aujourd'hui, le site et le service Internet sont à
`https://photocopie-benin.atinzed2.workers.dev`. Cette adresse marche, mais
elle est technique et contient votre identifiant. Un nom à vous
(par exemple `gestionphotocopie.com`) rassure les clients et les gérants.

**Rien ne casse pendant le changement** : l'ancienne adresse reste valable,
les QR déjà imprimés continuent de marcher, et les logiciels des boutiques
n'ont pas besoin de mise à jour.

## 1. Choisir et acheter le nom (vous, environ 10 à 15 dollars par an)

- Idées, à vérifier au moment de l'achat : `gestionphotocopie.com`,
  `gestion-photocopie.com`, `photocopie.app`. Un `.com` se lit partout, en
  français comme en anglais.
- Le plus simple : l'acheter **chez Cloudflare** (même compte que le site) :
  tableau de bord Cloudflare → *Domain Registration* → *Register Domains*.
  Cloudflare le vend à prix coûtant, et tout reste au même endroit.
- Payez avec une carte (ou une carte prépayée Visa/Mastercard).

## 2. Brancher le nom sur le service (5 minutes, depuis le tableau de bord Cloudflare)

1. *Workers & Pages* → **photocopie-benin** → *Settings* → *Domains & Routes* →
   *Add* → *Custom domain* → tapez votre nom (`gestionphotocopie.com`) → *Add*.
2. Attendez que l'état passe à *Active* (quelques minutes).

## 3. Dire au service d'utiliser ce nom

Dans `admin/wrangler.site.toml`, partie `[vars]` :

```
ADRESSE_PUBLIQUE = "https://gestionphotocopie.com"
ADRESSE_RELAIS = "https://gestionphotocopie.com"
```

Puis envoyez le changement sur GitHub (ou demandez-le à l'assistant) : le
déploiement est automatique.

Ce qui se passe ensuite, tout seul :

- le site utilise le nouveau nom dans ses liens, son plan et ses partages ;
- chaque logiciel de boutique l'apprend à sa prochaine inscription (au plus
  6 heures) : les **nouveaux** QR « Par Internet » portent le nouveau nom ;
- les anciens QR, avec l'adresse workers.dev, continuent de marcher.

## 4. Le prévenir aux moteurs de recherche

Dans Google Search Console, ajoutez le nouveau nom (*Ajouter une propriété*)
et envoyez son plan : `https://gestionphotocopie.com/sitemap.xml`.

## Si un jour vous changez d'hébergeur

Le service n'utilise que des interfaces web standard (voir `docs/passation.md`,
partie 2bis). Une fois le service installé ailleurs, mettez son adresse dans
`ADRESSE_RELAIS` **avant** d'arrêter l'ancien : les boutiques suivent toutes
seules en quelques heures, sans réimprimer leurs QR si le nom de domaine reste
le même.
