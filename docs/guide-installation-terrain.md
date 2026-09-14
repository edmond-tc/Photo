# Guide d'installation terrain (pour l'agent)

## Avant de partir
- Copier le fichier `.exe` (récupéré depuis GitHub Actions, voir README) sur
  une clé USB.
- Le fichier n'étant pas signé numériquement (coût à prévoir plus tard, cf.
  README), Windows va afficher un écran d'avertissement au premier
  lancement. **C'est normal, pas un bug.** Voir ci-dessous.

## Installation chez le gérant
1. Brancher la clé USB sur le PC de la boutique.
2. Copier le `.exe` sur le Bureau ou dans un dossier, puis double-cliquer
   dessus.
3. **Écran bleu-gris "Windows a protégé votre ordinateur"** : c'est ici que
   ça peut coincer.
   - Ne PAS cliquer sur "Ne pas exécuter" (gros bouton, trompeur).
   - Cliquer sur le petit lien **"Informations complémentaires"**.
   - Un bouton **"Exécuter quand même"** apparaît : cliquer dessus.
4. Suivre l'installeur (Suivant, Suivant, Installer).
5. Au premier lancement, l'assistant de bienvenue démarre automatiquement :
   - Choisir le dossier surveillé (créer un dossier dédié, ex.
     `Documents\Réception`, si besoin).
   - Entrer le nom de la boutique (et le WhatsApp si le gérant en a un pour
     la boutique).
   - Noter l'**identifiant machine** affiché à la dernière étape — c'est ce
     qu'il faudra pour générer sa clé de licence plus tard (voir README,
     section "Générer une clé de licence").
6. Vérifier avec le gérant, sur place :
   - Déposer un fichier test dans le dossier surveillé → il doit apparaître
     dans la file d'attente.
   - Scanner le QR code (bouton 📶) avec son propre téléphone → la page
     d'envoi doit s'ouvrir (peut nécessiter d'activer le partage de
     connexion Windows au préalable, bouton dans la fenêtre QR).
   - Imprimer un fichier test si une imprimante est branchée.

## Si l'antivirus du PC bloque ou met en quarantaine le fichier
Certains antivirus (pas seulement Windows Defender) peuvent réagir car le
logiciel est nouveau et fait des choses qu'un antivirus surveille de près
(serveur réseau local, surveillance de dossiers). Ce n'est pas un virus,
mais tant que le logiciel n'a pas de réputation établie (ou de signature de
code), ça peut arriver. Ajouter une exception dans l'antivirus si besoin.

## Ce qui ne dépend pas de vous
- Sur certaines cartes Wi-Fi anciennes, le PC ne peut pas être à la fois
  connecté à internet ET faire point d'accès (Mobile Hotspot). Si le
  partage de connexion ne s'active pas, le dossier surveillé et la clé USB
  restent utilisables normalement — ce n'est que la réception par QR qui
  serait affectée.
