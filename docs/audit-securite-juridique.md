# Audit de sécurité et audit juridique — 13 octobre 2026 (version 0.5.42)

Ce document répond à la question : « Où peut-on nous attaquer, en piratage
ou en justice, et que manque-t-il ? » Il est écrit en mots simples.

Il se lit en trois parties :

1. ce qui a été trouvé et **déjà corrigé** ;
2. ce qui a été vérifié et qui **tient** ;
3. ce qui **reste à faire par vous**, parce qu'aucun code ne peut le faire.

---

## 1. Failles trouvées et corrigées dans cette version

| Gravité | Problème | Ce qui pouvait arriver | Correction |
|---|---|---|---|
| **Grave** | Le bouton « Payer » du tableau de bord était bloqué par une règle de sécurité du navigateur (CSP `form-action`). | Le jour où FedaPay serait allumé, **aucun gérant n'aurait pu payer en ligne** : le navigateur refusait d'aller sur la page FedaPay. | Autoriser seulement `*.fedapay.com`. Vérifié dans un vrai Chrome (`admin/essais/paiement-navigateur.mjs`) : il échouait avant, il réussit après. |
| Moyenne | Nom de boutique avec des signes spéciaux (`$'`, `$&`) dans le titre de la page. | Une boutique pouvait abîmer sa propre page, ou y glisser du texte. | Remplacement sûr du titre. Vérifié. |
| Moyenne | Création de boutiques Internet sans limite. | Une seule personne pouvait créer des milliers de fausses boutiques et remplir la base. | Au plus 10 créations par heure et par adresse, et 500 par jour au total. Vérifié : la 11ᵉ création est refusée. |
| Moyenne | Le « lien du plan » d'une boutique acceptait n'importe quelle adresse. | Hameçonnage : une fausse boutique envoie les clients vers un faux site. | Seuls les sites de cartes sont acceptés (Google Maps, OpenStreetMap, Apple Plans, Waze, Bing). La règle est vérifiée deux fois : dans le logiciel et sur le relais. |
| Moyenne | Aucun moyen de signaler ni de fermer une page frauduleuse. | Une fausse « boutique » qui se fait passer pour une banque ou un opérateur reste en ligne. Juridiquement, il faut pouvoir agir dès qu'on est prévenu. | Ajout d'un lien « Signaler cette page » en bas de chaque page boutique (il ouvre WhatsApp). Dans le tableau de bord, la page *🌍 Boutiques Internet* a un bouton « ⛔ Fermer la page / ↩️ Rouvrir ». |
| Faible | Le lien « version classique » s'affichait aussi sur Internet, où il ne marche pas. | Lien cassé. | Le lien est caché sur Internet. |
| Moyenne | L'application Android pouvait être construite sur `main` avec la clé d'essai, qui est publique. | N'importe qui aurait pu publier une fausse mise à jour signée avec la même clé. | Sur `main`, la construction s'arrête si la vraie clé manque. |
| Juridique | Les textes ne disaient pas que les données passent par des serveurs hors du Bénin (Cloudflare, États-Unis compris). | Un manque d'information, que la loi reproche. | Ajouté dans les pages Confidentialité, en français et en anglais. |
| Juridique | Rien dans les conditions sur les pages frauduleuses. | Pas de base écrite pour fermer une page. | Ajout de « 4 bis. Pages frauduleuses et contenus illicites », en français et en anglais. |
| Juridique | Le gérant n'acceptait les conditions nulle part. | Les conditions sont plus faciles à contester. | Au premier lancement, le logiciel affiche : « En continuant, vous acceptez les conditions d'utilisation et la politique de confidentialité ». |
| Juridique | Les mentions légales demandaient un IFU que vous n'avez pas. | Des mentions fausses ou incomplètes. | L'éditeur est indiqué comme une personne physique, avec un directeur de la publication. L'IFU s'ajoutera tout seul quand vous l'aurez (`admin/site/pages-legal.js`, champ `immatriculation`). |

Tous les essais sont repassés après ces corrections :

- envoi par Internet : 20/20 ;
- charge : 60 commandes simultanées, rien de perdu ni de mélangé ;
- envoi en boutique : 50/50 ;
- paiement : délivrance des clés et des activations ;
- bouton « Payer » dans un vrai navigateur ;
- 284 tests du logiciel ;
- traductions complètes.

---

## 2. Ce qui a été vérifié et qui tient

- **Paiement** : un avis de paiement n'est jamais cru tout seul.
  - Il faut la signature, et un délai de moins de 5 minutes.
  - Le tableau de bord redemande ensuite la transaction à FedaPay : statut, montant, demande.
  - Une demande ne donne qu'une seule clé, même si l'avis arrive deux fois.
- **Licences** :
  - les clés sont signées et liées à un seul ordinateur ;
  - le code d'installation n'est donné qu'après paiement ou validation de votre part : pas d'activation gratuite ;
  - `/api/licence` ne rend qu'une clé déjà délivrée.
- **Tableau de bord** :
  - mot de passe limité en nombre d'essais ;
  - cookie de session protégé ;
  - la page ne peut pas être affichée dans une autre page (`frame-ancestors 'none'`) ;
  - le formulaire public de demande est limité en nombre d'envois.
- **Relais Internet** :
  - chaque boutique a sa clé secrète, et un ordinateur ne peut pas lire les commandes d'une autre boutique (testé en charge) ;
  - les documents sont effacés sous 24 heures au plus tard ;
  - limites en place : nombre de commandes par numéro, numéros bloqués, taille des fichiers.
- **Logiciel du gérant (PC)** :
  - seuls les types de documents autorisés sont ouverts, et ils sont marqués « venu d'Internet », pour que Windows et Office les ouvrent en mode protégé ;
  - une mise à jour non signée n'est jamais installée.
- **Application Android** :
  - le pont entre la page et l'application ne répond qu'à sa propre adresse ;
  - l'APK publié est signé avec la vraie clé.

**Limites connues, qu'on ne peut pas supprimer :**

- Un pirate habile peut toujours modifier un logiciel installé sur son propre ordinateur pour sauter la licence. C'est vrai de tous les logiciels. La parade est commerciale : un prix bas, de l'aide, des mises à jour.
- Le code est **public** sur GitHub : n'importe qui peut le lire. Ce n'est pas dangereux pour la sécurité, car aucun secret n'y figure : les clés sont dans Cloudflare et dans GitHub Secrets. En revanche, un concurrent peut copier vos idées. Si cela vous gêne : GitHub → *Settings* → *Change visibility* → *Private*. Les constructions automatiques continuent de marcher.

---

## 3. Ce qui reste à faire par vous (par ordre d'importance)

### A. Protéger vos comptes (à faire cette semaine — 1 heure)

1. **Double vérification** (code par SMS ou application) sur **GitHub**,
   **Cloudflare** et **Gmail**. Si quelqu'un vole un seul de ces comptes,
   il peut publier une fausse version du logiciel ou vider la base.
2. **Mot de passe du tableau de bord** (`ADMIN_PASSWORD`) : au moins 14
   caractères, utilisé nulle part ailleurs.
3. **Copies hors ligne des clés** : faites deux copies, sur deux clés USB rangées dans deux endroits différents.
   - La clé de licence (`CLE_PRIVEE_LICENCE`).
   - La clé de signature des mises à jour Windows.
   - La clé Android (le fichier `.jks` et son mot de passe).

   Une clé perdue, c'est la fin des mises à jour. Une clé volée, c'est des fausses licences ou de fausses mises à jour.
4. **Sauvegarde de la base** :
   - Cloudflare garde l'historique de la base D1 pendant 7 jours (offre gratuite) ou 30 jours (offre payante) — « Time Travel ».
   - Faites en plus une copie chaque mois : Cloudflare → *D1* → la base → *Export*, ou la commande `wrangler d1 export`.

### B. Être en règle au Bénin (avant d'encaisser beaucoup d'argent)

1. **APDP** (Autorité de protection des données personnelles). Le Code du numérique (loi n° 2017-20) demande de **déclarer** un traitement de données personnelles. Ici, le service traite les noms et numéros des clients et des gérants, et les documents.
   - Il faut aussi une **autorisation** pour envoyer des données hors du Bénin (serveurs Cloudflare).
   - Démarche : site apdp.bj, formulaire de déclaration. C'est souvent gratuit ou peu coûteux.
   - Mettez ensuite le numéro de récépissé dans les pages Confidentialité.
2. **Statut fiscal** : vendre des licences est une activité commerciale.
   - Il faut un **IFU** et un statut : « entreprenant », ou entreprise individuelle au RCCM. Les deux se demandent au guichet unique (APIEx / monentreprise.bj).
   - FedaPay le demandera aussi pour un compte marchand « live ».
   - Une fois l'IFU obtenu, ajoutez-le aux mentions légales (voir plus haut).
3. **Marque** : déposer « Gestion Photocopie » (nom et logo) à l'**OAPI**. Un seul dépôt couvre les 17 pays membres, dont le Bénin, le Togo, la Côte d'Ivoire, le Sénégal et le Cameroun. Sans dépôt, quelqu'un peut déposer le nom avant vous.
4. **Relecture par un juriste** des conditions et de la confidentialité. Elles sont claires et honnêtes, mais n'ont pas été relues par un professionnel. Un avocat béninois, pour une ou deux heures, suffit.

### C. Avant d'aller dans d'autres pays

Chaque pays a sa propre loi sur les données personnelles. Avant d'y vendre :

| Pays | Démarche |
|---|---|
| **Ghana** | S'enregistrer auprès de la Data Protection Commission (Data Protection Act 2012). |
| **Nigeria** | Se renseigner auprès de la NDPC (Nigeria Data Protection Act 2023). |
| **Kenya** | Enregistrement auprès de l'ODPC (Data Protection Act 2019). |
| **Côte d'Ivoire** | ARTCI. |
| **Sénégal** | CDP. |
| **Togo** | IPDCP. |

Dans ces pays, le paiement passera par un autre intermédiaire que FedaPay (Paystack, Flutterwave) : leur ouverture de compte demandera aussi une entreprise.

### D. Essais à faire en vrai (aucun ordinateur ne peut les faire à votre place)

1. Un **vrai paiement test FedaPay** : mode « test », clés posées dans Cloudflare (voir `docs/paiements.md`).
2. Un envoi depuis un **vrai iPhone** et un **vrai téléphone Android bas de gamme**, en boutique et par Internet.
3. Un essai pilote dans **2 ou 3 boutiques** pendant 2 à 4 semaines avant de vendre largement.

### E. Que faire en cas de problème

- **Une page boutique est signalée** : tableau de bord → *🌍 Boutiques Internet* → « ⛔ Fermer la page ». Gardez le message de signalement.
- **Une autorité demande des informations** : ne répondez qu'à une demande écrite et officielle. Notez la date et ce que vous avez remis. Le relais ne garde presque rien : les documents sont effacés sous 24 heures et les suivis sous 7 jours.
- **Un compte est volé** (GitHub, Cloudflare) :
  - changez les mots de passe ;
  - dans Cloudflare, changez `ADMIN_PASSWORD` ;
  - regardez les dernières publications sur GitHub (*Actions*).
- **Une clé est volée** : prévenez-moi. Il faut alors une nouvelle clé et une mise à jour pour tous.
