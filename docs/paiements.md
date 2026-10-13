# Paiements : ce qui marche déjà, et comment allumer le paiement en ligne

## Ce qui marche dès maintenant (sans rien faire)

- **La clé arrive toute seule dans le logiciel.** Dès que vous confirmez une
  demande dans le tableau de bord (« ✅ Confirmer et générer la clé »), le
  logiciel de la boutique la récupère par Internet, dans les 10 minutes si
  son abonnement se termine (6 heures sinon). Le gérant voit « ✓ Abonnement
  renouvelé ». Plus besoin de lui envoyer la clé par WhatsApp. (La clé reste
  affichée dans le tableau de bord, pour une boutique sans connexion : clé
  USB ou saisie, comme avant.)
- **Payer en avance ne fait rien perdre** : la nouvelle période s'ajoute à
  la fin de l'actuelle.

## Le paiement en ligne (Mobile Money et cartes) : ce qu'il faut de votre part

Le code est prêt et testé contre un faux FedaPay (`admin/essais/paiement.mjs`) :
il reste **éteint** tant que vos clés FedaPay ne sont pas posées.

### 1. Ouvrir un compte FedaPay (vous)

- Sur fedapay.com, créez un compte marchand **à votre nom** (ou au nom de
  votre entreprise). FedaPay vous demande vos pièces : pièce d'identité, et
  selon votre statut, IFU ou RCCM. Il vous donne la liste exacte.
- FedaPay encaisse MTN, Moov et les cartes, au Bénin et dans plusieurs pays
  voisins, et vous reverse l'argent moins sa commission.

### 2. Essayer sans argent (mode « test »)

1. Dans l'espace FedaPay, mode **test** : copiez la **clé secrète** (elle
   commence par `sk_sandbox_` ou `sk_test_`).
2. Créez un **webhook** (avis de paiement) vers
   `https://photocopie-admin.atinzed2.workers.dev/paiement/fedapay`, pour les
   événements de transaction (au moins « transaction approuvée »). Copiez son
   **secret**.
3. Cloudflare → *Workers & Pages* → **photocopie-admin** → *Settings* →
   *Variables and Secrets* → *Add* (type **Secret**) :
   - `FEDAPAY_CLE_SECRETE` = la clé secrète ;
   - `FEDAPAY_SECRET_WEBHOOK` = le secret du webhook.
4. Tableau de bord → *Paramètres* → partie « Paiement en ligne » : mettez le
   prix de chaque durée (30, 90, 365 jours). Une durée sans prix n'est pas
   proposée.
5. Ouvrez `/renouveler` : le bloc « 💳 Payer en ligne maintenant » apparaît.
   Payez avec un numéro de test FedaPay : la demande se ferme toute seule et
   la clé est délivrée.

### 3. Passer aux vrais paiements

Refaites l'étape 2 avec les clés du mode **live**, puis ajoutez la variable
(type **Text**, pas Secret) `FEDAPAY_MODE` = `live`.

### Sûreté

- Un avis de paiement n'est cru que s'il porte la bonne signature (moins de
  5 minutes, secret du webhook).
- Même signé, il ne suffit pas : le tableau de bord redemande la transaction
  à FedaPay et vérifie le statut « approuvé », le montant et la demande.
- Une demande ne délivre qu'une seule clé, même si deux avis arrivent.
- Si un avis se perd, la page de retour du gérant vérifie aussi, et le
  tableau de bord montre les paiements commencés non confirmés, avec un
  bouton « 🔎 Vérifier auprès de FedaPay ».

## Plus tard

- **Ghana, Nigeria** : FedaPay n'y est pas ; Paystack ou Flutterwave s'y
  branchent de la même façon (un second fichier comme `admin/src/paiement.js`).
- **Le client qui paie sa commande avant l'impression** : c'est un autre
  modèle (l'argent va à la boutique, pas à vous). Chaque boutique devrait
  avoir son propre compte, ou vous encaisseriez pour elles et reverseriez :
  c'est une décision commerciale et réglementaire à prendre avant d'écrire
  le code.
