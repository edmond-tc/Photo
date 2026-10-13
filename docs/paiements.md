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

## Ce que vous encaissez

Seulement deux choses, payées par le gérant de la boutique :

1. **Les frais d'activation**, une fois, à l'installation.
2. **La licence** (l'abonnement : 30, 90 ou 365 jours).

Le client de la boutique paie ses photocopies **au guichet**, à la
boutique : cet argent ne passe jamais par vous, et le logiciel n'y touche pas.

**Activation sans rien recopier** : l'écran « Installation à valider » du
logiciel vérifie toutes les 20 secondes si son code a été délivré, et
s'active tout seul. Le code est délivré quand vous validez l'installation
dans le tableau de bord (*🔒 Installations*), ou dès que le gérant a payé
l'activation en ligne (QR affiché sur ce même écran). Sans paiement ni
validation, aucun code n'est donné : pas d'activation gratuite.

**Renouvellement** : l'écran « Accès suspendu » affiche lui aussi un QR de
paiement en ligne ; la nouvelle clé arrive ensuite toute seule (vérifié
toutes les 3 minutes).

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
4. Tableau de bord → *Paramètres* → partie « Paiement en ligne » : mettez les
   **frais d'activation** et le prix de chaque durée (30, 90, 365 jours). Un
   prix vide = pas proposé en ligne.
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

## Paiement lent (Mobile Money qui confirme tard)

Il arrive que le paiement mette plusieurs minutes à être confirmé. Ce délai
vient le plus souvent de l'opérateur (MTN, Moov) : le client valide le code
sur son téléphone, puis l'opérateur prévient FedaPay. Le logiciel est prévu
pour ça :

- la page « Paiement en attente » se met à jour toute seule toutes les
  10 secondes pendant 15 minutes : le gérant n'a rien à faire ;
- même si le gérant ferme cette page et que l'avis de FedaPay se perd, le
  logiciel de la boutique, en venant chercher sa clé (toutes les 20 s pour
  l'activation, toutes les 3 min quand l'accès est suspendu), fait vérifier
  le paiement auprès de FedaPay : la clé arrive dès la confirmation, jusqu'à
  2 jours après ;
- la page dit au gérant de **ne pas payer une deuxième fois** si l'argent a
  été retiré.

Pour savoir si FedaPay est assez rapide aujourd'hui : en mode **live**, mettez
un prix de 100 FCFA, payez vous-même 3 ou 4 fois (MTN et Moov), et notez le
temps entre la validation du code et « ✅ Paiement reçu ». Remettez ensuite le
vrai prix. Si c'est trop lent ou trop souvent en échec, une autre passerelle
béninoise (par exemple KkiaPay) se branche de la même façon : un second fichier
comme `admin/src/paiement.js`.

## Plus tard

- **Ghana, Nigeria** : FedaPay n'y est pas ; Paystack ou Flutterwave s'y
  branchent de la même façon (un second fichier comme `admin/src/paiement.js`).
- **Le paiement des commandes des clients** : décidé le 13 octobre 2026 —
  pas prévu. Le porteur du projet n'encaisse que l'activation et la licence ;
  les clients paient la boutique au guichet.
