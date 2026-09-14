# Cahier des charges — Logiciel de gestion pour boutique de photocopie (Bénin)

## Message pour Claude Code

Tu es chargé de construire un logiciel de bureau (Windows) pour les gérants de boutiques de photocopie au Bénin. Ce document contient TOUT ce qui a été validé avec le porteur du projet après des semaines d'échanges et de recherche terrain (visites réelles de boutiques). Lis-le en entier avant de commencer. Là où le document te laisse une marge de décision, choisis la solution la plus simple et la plus robuste — le principe directeur numéro un est : **simplicité radicale pour l'utilisateur final, qui n'est pas technophile**.

---

## 1. Contexte et vision

Le porteur du projet a déjà un produit web (Cloudflare Worker) qui connecte boutiquiers et clients via QR code et paiement. Le retour terrain (visites réelles de boutiques) a révélé un problème récurrent : **les gérants n'arrivent pas à partager la connexion internet de leur PC**, et doivent utiliser leurs propres données mobiles à chaque réception de fichier client par WhatsApp. Ce nouveau logiciel doit résoudre ce problème en étant **100% fonctionnel sans connexion internet**.

Objectif produit : un logiciel que le gérant peut adopter **sans grande formation**, qui **réduit la charge de travail** du gérant ET du client, et qui devient **incontournable** dans son usage quotidien — pas un gadget que personne n'utilise après la démo.

Le porteur vise un déploiement dans tout le Bénin, potentiellement au-delà (zone CFA/Afrique de l'Ouest) dans un second temps.

---

## 2. Principes directeurs (non négociables)

1. **100% hors ligne** — aucune fonctionnalité cœur ne doit dépendre d'internet. Réception de fichiers, conversion, dashboard, impression : tout fonctionne sans connexion.
2. **Un seul écran principal, pas d'onglets visibles en permanence.** Le gérant doit voir et faire l'essentiel (recevoir → prévisualiser → imprimer) sans naviguer. Les fonctions secondaires (historique, rapports, réglages) sont rangées dans un **menu à trois points (⋮) ou hamburger, ouvrant un panneau latéral à droite** (inspiré de l'interface Claude.ai : un clic ouvre une liste de sections, pas des onglets permanents).
3. **Rapidité maximale pour le gérant** — chaque demande client doit pouvoir être traitée en un minimum de clics.
4. **Paiement d'installation + essai gratuit + abonnement mensuel bas, sans prix affiché dans l'application** — voir section 6bis pour le détail complet du modèle économique révisé.
5. **Léger et simple à construire** — préférer des briques technologiques matures et éprouvées à des solutions maison complexes.

---

## 3. Architecture technique retenue

- **Application de bureau** : Tauri (cœur Rust + interface HTML/CSS/JS) — choisi pour sa légèreté (exécutable de quelques dizaines de Mo, contre 150-200 Mo pour Electron) et parce qu'il permet de réutiliser des compétences web déjà mobilisées sur le projet Cloudflare Worker existant.
- **Base de données locale** : SQLite (fichier local, aucun serveur, résiste bien aux écritures interrompues si utilisée en mode WAL — important vu les coupures de courant fréquentes au Bénin).
- **Réseau local** : le PC crée son propre point d'accès Wi-Fi (Windows Mobile Hotspot), sans passer par un routeur ni par internet.
- **Réception des fichiers côté client** : un QR code affiché en boutique pointe vers une petite page web servie localement par le logiciel (serveur local intégré à l'appli Tauri). Le client scanne, se connecte au Wi-Fi de la boutique, une page s'ouvre dans son navigateur (**aucune application à installer côté client**), il choisit son fichier et l'envoie.
- **Affichage instantané côté gérant** : une connexion WebSocket maintenue entre le serveur local et le dashboard pousse chaque nouveau fichier reçu (QR ou dossier surveillé USB) directement dans la file d'attente, **sans rechargement de page**.
- **Traitement des fichiers — décision finale (révisée) : router vers les outils déjà connus du gérant plutôt que tout automatiser en interne.** Objectif : éviter que le logiciel devienne lui-même un obstacle nécessitant une formation. Le rôle du logiciel se limite à *recevoir* les fichiers et les mettre en file d'attente rapidement — pas à réinventer l'édition ou l'impression.
  - **Si le fichier est déjà un PDF** (ou une image) → le bouton "Imprimer" ouvre directement la **boîte de dialogue d'impression standard de Windows** sur ce fichier. Le gérant imprime exactement comme il le fait déjà d'habitude, aucune interface propriétaire à apprendre.
  - **Si le fichier nécessite une édition, une mise en forme, ou une saisie** (Word, PowerPoint, Excel, document à taper) → un bouton "Ouvrir/Éditer" lance directement **l'éditeur que le gérant utilise déjà** (Microsoft Word/Office s'il est installé — cas majoritaire d'après le terrain — sinon LibreOffice Writer/Impress/Calc en solution de secours). C'est cet éditeur natif qui fait le travail d'édition ET l'impression, pas notre logiciel.
  - **Exigence de performance impérative** : le clic sur "Ouvrir/Éditer" doit lancer l'éditeur **le plus rapidement possible**, sans latence perceptible — le gérant ne doit jamais sentir d'attente entre son clic et l'ouverture réelle du fichier.
- Ancienne piste abandonnée : l'automatisation COM invisible (Word piloté en arrière-plan sans jamais s'afficher) n'est plus retenue — trop de complexité et de risque de blocage pour un bénéfice marginal, alors que le gérant est de toute façon déjà à l'aise avec Word/LibreOffice au quotidien.
- **Lecture d'état de l'imprimante** (niveau d'encre/toner, compteur de pages) : via SNMP pour les imprimantes réseau (fiable et standard) ; pour les imprimantes USB simples, tenter une lecture via les pilotes du fabricant si disponible, sans bloquer le fonctionnement si l'info n'est pas accessible (dégradation silencieuse, pas d'erreur affichée au gérant).
- **Scanner** (numérisation) : intégration via les standards TWAIN/WIA de Windows, universels à tous les fabricants.

---

## 4. Fonctionnalités — Cœur (couche de base)

### Réception des fichiers (3 canaux, tous alimentent la même file d'attente unique)
- QR code / Wi-Fi local (décrit ci-dessus)
- Clé USB branchée
- Dossier surveillé automatiquement sur le PC (glisser-déposer)

### Traitement
- Aperçu miniature de chaque fichier en attente, avec nom du client (si fourni via le formulaire QR) et heure de réception
- Détection du type de fichier et routage automatique : PDF/image → dialogue d'impression Windows natif ; fichier éditable → ouverture dans Word/LibreOffice (voir section 3)
- Options d'impression visibles directement (pas de sous-menu caché) : nombre de copies, N&B/couleur, format papier (A4/A3/A5), recto-verso, orientation
- Bouton "Imprimer" toujours visible et évident
- **Aperçu avant impression obligatoire** pour éviter le gaspillage de papier en cas de fichier corrompu ou mal formaté

### Finitions (métadonnées à afficher au gérant, exécution physique manuelle)
Le logiciel ne peut pas piloter une agrafeuse ou une plastifieuse — il doit simplement enregistrer et afficher clairement la demande du client : agrafage, reliure spirale, reliure dos carré collé, plastification, découpe/massicotage, perforation, pliage.

### Service de saisie/dactylographie
Un éditeur de texte simple intégré (mini-traitement de texte) pour les cas où le client apporte un document manuscrit ou une idée à taper depuis zéro, plutôt qu'un fichier déjà prêt — service confirmé comme réel et fréquent par la recherche terrain (ex. CESIE Bénin).

### Gestion des cas problématiques
- Fichier corrompu → message clair au gérant, pas de plantage du logiciel
- Format non supporté (.pages, .zip, etc.) → message explicite indiquant le format reçu et invitant à redemander un format standard au client
- PDF protégé par mot de passe → demander le mot de passe au gérant/client avant traitement
- Fichier trop volumineux → indicateur de progression clair pendant le traitement
- Mauvais format/orientation d'origine (ex. US Letter au lieu de A4) → détection et proposition d'ajustement automatique à l'aperçu

### Numéro de téléphone (si formulaire client avec numéro)
Réforme béninoise du 30/11/2024 : les numéros ont 10 chiffres avec le préfixe **"01"** qui fait partie intégrante du numéro (ce n'est PAS un indicatif de tronc à retirer). Logique de normalisation validée sur le produit web existant, à reprendre telle quelle :

```js
function normalizePhone(raw) {
  let p = (raw || "").replace(/[^0-9]/g, "");
  if (p.startsWith("229")) p = p.slice(3);
  if (p.length === 8) p = "01" + p; // ancien numéro à 8 chiffres sans le préfixe 01
  return "229" + p;
}
```

---

## 5. Fonctionnalités — Gestion (couche premium)

Ces fonctions justifient un positionnement tarifaire supérieur à la couche de base — elles apportent une valeur continue (pas juste "imprimer plus vite").

- **Suivi financier** : chiffre d'affaires calculé automatiquement (jour/semaine/mois) à partir des impressions facturées, suivi des dépenses (papier, encre, électricité), bénéfice net calculé automatiquement
- **Grille tarifaire personnalisable** : chaque boutique règle ses propres prix ; calculatrice de prix intégrée (le gérant compose la commande, le prix sort automatiquement)
- **Génération de reçu/facture** simple, imprimable
- **Gestion de stock** : suivi du papier et de l'encre/toner restants, alerte automatique en cas de stock bas, compteur de pages depuis le dernier changement de cartouche
- **Historique et recherche client** : retrouver une ancienne commande, suivi des habitués
- **Rapport de fin de journée automatique** : résumé généré seul (nombre de clients, total encaissé, service le plus demandé)
- **Maintenance imprimante** : compteur d'utilisation, alerte d'entretien préventif
- **Suivi multi-employés** (si plusieurs personnes travaillent dans la boutique) : qui a traité quelle commande
- **Export des rapports** en PDF/Excel
- **Sauvegarde automatique locale** de la base de données (vers un dossier local ou une clé USB) — essentiel vu le risque de coupure de courant

### Organisation du menu latéral (⋮ / panneau droit)
- Commandes en cours (aussi visible sur l'écran principal)
- Historique
- Documents reçus (classés par client/date)
- Rapports
- Réglages (grille tarifaire, couleurs de l'interface, infos boutique, licence, mises à jour)

### Personnalisation visuelle
Pour cette version : **pas de personnalisation de couleur**. L'interface garde une couleur unique fixe, aussi bien côté dashboard gérant que côté page web client (QR). Style visuel proche de Microsoft Office (voir section 5bis) pour la familiarité, sans option de personnalisation pour l'instant — ça pourra être ajouté dans une version ultérieure si demandé.

---

## 5bis. Style visuel et fonctionnalités d'encaissement avancées

### Style visuel — familiarité Microsoft Office
Pour réduire au minimum la courbe d'apprentissage, l'interface doit visuellement rappeler l'environnement Microsoft Office que les gérants utilisent déjà au quotidien :
- Police : **Calibri** ou **Segoe UI** (polices par défaut d'Office/Windows)
- Icônes et boutons dans un style sobre et reconnaissable (rectangles nets, pictogrammes simples : imprimante, disquette, loupe) plutôt qu'un design "app moderne" abstrait
- Couleurs de base neutres (gris clair, bleu Office classique) — voir aussi la section 5 sur l'absence de personnalisation pour cette version

### Fonctionnalités d'encaissement (couche Gestion/premium)
En plus du suivi financier déjà listé en section 5, ajouter :
- **Intégration Mobile Money** (Orange Money, MTN MoMo — moyens de paiement dominants dans la sous-région) : enregistrement d'un paiement par Mobile Money directement dans le logiciel, pas seulement en espèces
- **Suivi multi-moyens de paiement** : espèces / Mobile Money / à crédit, avec état clair des impayés
- **Clôture de caisse en fin de journée** : total encaissé réel vs total attendu selon les commandes traitées, pour repérer tout écart immédiatement
- **Rappel des impayés** : liste des clients qui doivent encore régler, avec relance suggérée
- **Tarifs dégressifs/fidélité automatique** : réduction automatique selon volume ou client régulier, sans calcul manuel
- **Reçu/facture personnalisé** avec logo et nom de la boutique

### Collecte du numéro de téléphone (formulaire QR client)
Le numéro de téléphone collecté lors du scan QR par le client sert **uniquement** à permettre au porteur du projet de recontacter les personnes plus tard (suivi commercial/marketing) — aucune autre utilisation fonctionnelle dans le logiciel lui-même.

### Flux de réception QR — notre solution mise en avant, alternatives disponibles en option secondaire
La page web ouverte après le scan du QR code doit mettre en avant, **en haut, avec un gros bouton bien visible**, l'envoi via notre interface web hébergée localement — c'est le chemin que le client doit voir et choisir en premier. Les options **WhatsApp et Bluetooth restent disponibles, mais en bas de la page**, en petits liens secondaires plutôt qu'en boutons de même taille — pour ne pas les cacher complètement (certains clients y tiendront), mais sans jamais les mettre en concurrence visuelle avec notre solution, qui doit clairement dominer l'écran.

## 6. Distribution et mises à jour (sans dépendre d'agents permanents sur le terrain)

- **Installation initiale** : clé USB (méthode principale, fiable, ne dépend de rien), avec option de téléchargement en ligne en complément pour les boutiquiers ayant une connexion correcte.
- **Mises à jour** : le logiciel affiche discrètement son numéro de version sur le dashboard. Deux mécanismes, sans jamais nécessiter la présence d'un agent :
  1. Le gérant télécharge la mise à jour avec son propre téléphone (lien fourni), puis la transfère vers son PC via **le même système Wi-Fi local déjà construit pour recevoir les fichiers clients** (bouton "Mettre à jour" réservé au gérant dans les réglages).
  2. Vérification opportuniste en arrière-plan si une connexion internet ponctuelle est disponible — affiche juste un badge discret "mise à jour disponible", sans jamais forcer l'installation.
- Fermer le logiciel avant une mise à jour est normal et sans risque : la base de données (historique, clients) est un fichier séparé du programme, jamais affecté par le remplacement du programme.

---

## 6bis. Modèle économique (révisé — implications pour le produit)

Le modèle de prix a été révisé pendant les échanges avec le porteur du projet ; ceci a des conséquences directes sur ce que le logiciel doit permettre :

- Paiement d'installation minimum obligatoire (2000 FCFA), puis **1 mois d'essai gratuit sur toutes les fonctionnalités** (y compris la couche Gestion/premium), puis **facturation mensuelle** à un prix bas ajusté par le porteur selon son volume d'adoption.
- **Aucun prix ne doit jamais être affiché dans l'interface du logiciel.** Le prix est communiqué et négocié en dehors de l'application (par le porteur/ses agents directement avec le boutiquier).
- Le logiciel doit donc savoir gérer un **statut d'abonnement** (essai gratuit actif / abonnement payant actif / expiré) sans jamais afficher de montant en FCFA à l'écran — uniquement un état ("essai gratuit — X jours restants", "abonnement actif", ou un écran de blocage neutre en cas d'expiration invitant à contacter le porteur, sans prix affiché).
- Le mécanisme de licence (section 7) doit donc supporter une vérification de date/statut, pas seulement une activation unique à vie.

## 7. Licence et anti-piratage (point identifié, à ne pas négliger)

Comme le logiciel est distribué hors ligne sans vérification serveur, rien n'empêche nativement la copie non autorisée du programme d'un PC à un autre. Prévoir un mécanisme léger : génération d'une **clé de licence liée à des caractéristiques du PC** (ex. identifiant matériel), saisie une fois à l'installation, sans nécessiter de connexion internet pour la vérifier (vérification locale par calcul cryptographique, pas d'appel à un serveur). Le porteur du projet distribue lui-même les clés lors de l'installation en boutique.

---

## 8. Robustesse et résilience (points anticipés, à traiter dès la conception)

- **Coupures de courant fréquentes au Bénin** : utiliser SQLite en mode WAL (Write-Ahead Logging) pour limiter le risque de corruption de données en cas d'extinction brutale ; sauvegardes automatiques régulières.
- **Antivirus/SmartScreen Windows** : un exécutable non signé numériquement peut être signalé comme suspect au premier lancement. Prévoir des instructions claires (avec captures d'écran) pour guider le gérant/l'agent à travers cet écran d'avertissement ; envisager une signature de code si le budget le permet à terme.
- **Wi-Fi déjà utilisé autrement** : certaines cartes Wi-Fi anciennes ne peuvent pas faire "point d'accès" et "connexion internet" simultanément — détecter l'échec de création du point d'accès et afficher un message clair plutôt qu'un plantage silencieux.
- **Diversité des configurations Windows** (7/8/10/11, copies non originales) : tester sur plusieurs versions ; dégrader proprement (message d'erreur clair) plutôt que planter si une fonctionnalité n'est pas disponible sur une configuration donnée.
- **Portée Wi-Fi limitée** dans les grandes boutiques : documenter la possibilité d'ajouter un petit point d'accès Wi-Fi externe bon marché en cas de besoin, sans que ce soit obligatoire au départ.

---

## 9. Ce qui est explicitement HORS PÉRIMÈTRE pour une V1

- Tableau de bord central pour le porteur du projet (vue sur toutes les boutiques, gestion des licences à distance) — utile plus tard si le volume de boutiques le justifie, mais chaque installation doit rester totalement indépendante et fonctionnelle sans lui.
- Pilotage physique des machines de finition (agrafeuse, plastifieuse, spirale) — technologiquement hors de portée et hors sujet, exécution manuelle par le gérant.
- Application mobile dédiée — le client n'a besoin que d'un navigateur, pas d'app à installer.

---

## 10. Suggestions supplémentaires (anticipées, non discutées explicitement avec le porteur — à proposer, pas à imposer)

Ces idées visent à renforcer l'adoption et la robustesse ; à confirmer avec le porteur du projet avant développement si un doute existe sur la priorité :

- **Mode démo/formation intégré** : un jeu de données factices pour que l'agent terrain puisse faire une démonstration sans avoir besoin d'un vrai client sur place lors du démarchage.
- **Raccourcis clavier** pour les actions les plus fréquentes (imprimer, copies +/-) — utile pour un gérant qui traite beaucoup de commandes par jour.
- **File d'attente triable** (par heure d'arrivée, par urgence signalée par le client) plutôt que strictement chronologique.
- **Notification sonore discrète** à l'arrivée d'un nouveau fichier, pour que le gérant n'ait pas besoin de garder l'œil rivé sur l'écran en permanence.
- **Mode "boutique fermée temporairement"** : désactiver l'acceptation de nouveaux fichiers via le QR sans éteindre le PC (ex. pendant une coupure de courant, une pause).
- **Estimation du coût papier/encre par tâche**, affichée au gérant avant impression, pour affiner sa marge en temps réel.

---

## 11. Plan de développement suggéré (par étapes, pas tout en une fois)

- Dès le tout début du projet, mettre en place un **workflow GitHub Actions** qui compile automatiquement le projet Tauri en exécutable Windows (.exe) à chaque envoi de code, sur une machine Windows fournie gratuitement par GitHub — nécessaire car le développement se fait depuis un environnement cloud Linux (Claude Code sur le web), qui ne peut pas fabriquer nativement un .exe Windows. Le fichier .exe compilé doit être téléchargeable directement depuis la page du workflow GitHub Actions, pour être récupéré et testé sur un vrai PC Windows.

1. Squelette Tauri + SQLite + écran unique avec réception de fichier via dossier surveillé (le plus simple, sans réseau)
2. Détection PDF vs fichier éditable + routage (dialogue d'impression Windows natif, ou ouverture dans Word/LibreOffice) + aperçu
3. Serveur local + QR code + réception via Wi-Fi local + WebSocket pour affichage instantané
4. Options d'impression complètes + gestion des finitions (métadonnées)
5. Couche Gestion (suivi financier, stock, rapports)
6. Menu latéral (panneau droit), style visuel Office, réglages
7. Licence, mises à jour, robustesse (coupures de courant, tests multi-Windows)

À chaque étape : tester réellement sur la machine de développement avant de passer à la suivante, plutôt que de tout coder d'un bloc sans jamais lancer le programme.
