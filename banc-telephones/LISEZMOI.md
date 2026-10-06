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
