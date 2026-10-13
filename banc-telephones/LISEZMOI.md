# Banc d'essai des téléphones (sans téléphone)

La vraie page du client (`client-web/index.html`), ouverte dans le vrai moteur
Chrome d'Android, en imitant dix téléphones courants au Bénin (Tecno, Infinix,
itel, Samsung, Redmi, Huawei, Oppo, petit écran), face à un faux PC de
boutique qui répond comme `server.rs`.

Cinq situations pour chaque téléphone :

1. envoi normal d'un document ;
2. la petite fenêtre « Se connecter au réseau » d'Android ;
3. le téléphone part sur ses données mobiles en plein envoi, puis revient ;
4. le PC injoignable à l'ouverture de la page, puis joignable ;
5. la liaison saute pile à la validation de la commande.

Lancer : `node banc-telephones/banc.mjs` (Playwright et Chromium installés).

Ce que ce banc ne remplace pas : le Wi-Fi réel, le système Android lui-même
(choix entre Wi-Fi et données mobiles), Safari sur iPhone. Pour cela, un
essai en boutique reste nécessaire.

## Par Internet, plusieurs boutiques en même temps

`charge-relais.mjs` : plusieurs boutiques inscrites sur le relais, et pour
chacune beaucoup de clients qui envoient en même temps (documents de 0,5 à
4 Mo). Un client sur cinq imite un réseau qui hésite : le même morceau part
deux fois, et le bouton « Réessayer » est touché après la commande. Le faux
PC de chaque boutique récupère ses commandes. Le banc vérifie que chaque
boutique reçoit exactement SES commandes, intactes, une seule fois, et que
le client voit le numéro donné par le guichet.

    RELAIS=http://127.0.0.1:8787 BOUTIQUES=6 CLIENTS=20 node banc-telephones/charge-relais.mjs

Il tourne aussi, plus petit (3 boutiques × 10 clients), contre le vrai
relais en ligne après chaque déploiement (`.github/workflows/essai-relais.yml`).
