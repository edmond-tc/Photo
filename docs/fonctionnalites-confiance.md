# Fonctionnalités de confiance — décisions et backlog

Document de suivi des échanges avec le porteur du projet sur "qu'est-ce qui
rend le logiciel indispensable, pas juste pratique" — au-delà des
fonctionnalités déjà listées dans le cahier des charges.

## Principe retenu (après plusieurs itérations)

Ne pas verrouiller l'accès avec des codes PIN (décision explicite : "oublie
code pin" — trop de friction, un code peut se partager/deviner). À la
place : **rendre impossible de faire disparaître quelque chose
silencieusement**. Chaque fait s'enregistre automatiquement et ne peut
jamais être modifié ou supprimé après coup — la confiance vient de la
traçabilité totale, pas d'un mot de passe.

## Implémenté (cette itération)

- **Réconciliation fichiers reçus vs traités** : chaque fichier qui arrive
  (dossier surveillé, USB, QR) est enregistré automatiquement dès son
  arrivée, avant tout geste humain — ce compteur ne peut pas être
  contourné. "Ignorer un fichier" exige désormais une raison obligatoire
  (format non supporté, doublon, client absent...), enregistrée. Un
  rapport de réconciliation compare fichiers reçus / payés / ignorés
  (avec raisons) / en attente.
- **Montant calculé vs montant réellement encaissé** : le prix calculé par
  la grille tarifaire est toujours enregistré à côté du montant final. Si
  le gérant modifie le montant proposé, une raison est obligatoire et les
  deux valeurs restent visibles dans l'historique — jamais un simple
  écrasement silencieux.
- **Historique des changements de tarifs** : chaque modification de prix
  (ancien prix, nouveau prix, date) est journalisée de façon permanente.
- **Garantie "aucune suppression"** : ni les fichiers de la file, ni les
  transactions, ni les changements de tarifs ne peuvent être supprimés
  depuis l'application — aucune fonction de suppression n'existe pour ces
  données. Seul le mode démonstration peut être retiré (ce sont des
  données factices, pas de vraies transactions).
- **Numérotation continue des reçus** : chaque reçu imprimé porte le
  numéro de transaction (auto-incrémenté, jamais réutilisé) — un propriétaire
  qui inspecte une pile de reçus remarque immédiatement un trou dans la
  séquence.

## Backlog (idées validées, pas encore construites — à prioriser plus tard)

- Badge "commande oubliée" si un fichier reste en attente trop longtemps.
- Ticket de fin de journée imprimable ("ticket Z") à remettre physiquement
  au propriétaire.
- Bouton "Sauvegarder maintenant" visible sur l'écran principal (au-delà
  de la sauvegarde automatique en tâche de fond).
- Alerte si aucune sauvegarde vers un support externe (clé USB dédiée)
  depuis plusieurs jours — la sauvegarde actuelle protège des coupures de
  courant, pas du vol/casse du PC lui-même.
- Fond de caisse déclaré en début de journée, pour une clôture de caisse
  complète (fond de départ + ventes - dépenses = attendu en fin de
  journée).
- Historique des écarts de caisse dans le temps, par employé (tendance,
  pas juste un chiffre isolé).
- Vue "cette semaine / ce mois" dans Rapports, pas seulement "aujourd'hui".
- Heures de pointe (nombre de clients par heure) — utile pour la
  planification du personnel, notamment en période d'examens.
- **"Nouveautés de cette mise à jour"** : à chaque renouvellement mensuel,
  montrer au gérant un résumé simple et concret de ce qui a changé/gagné
  depuis sa version actuelle (langage bénéfice, pas jargon technique) —
  façon "Quoi de neuf" de Word. But : que le renouvellement se ressente
  comme un investissement compris, pas une dépense subie. S'appuie sur le
  mécanisme déjà existant (fichier de mise à jour déposé dans le dossier
  surveillé → apparaît comme "Installer la mise à jour" dans la file) —
  il manque l'écran qui explique les nouveautés avant/pendant ce clic.
  Techniquement : tenir un petit fichier de changelog (par version) que le
  porteur du projet remplit à chaque envoi de mise à jour, affiché dans
  l'appli au moment de l'installation et/ou sur la page `/renouveler` du
  tableau de bord pour renforcer la valeur perçue avant paiement.

## Idées explorées puis écartées

- **Codes PIN propriétaire/employé** : écarté par décision explicite du
  porteur du projet (friction inutile, un code peut se partager).
- **Lecture du journal d'impression Windows** (compteur de pages réel) :
  écarté — ne couvre pas les fichiers ouverts dans Word (l'impression se
  fait alors depuis Word, hors de portée de l'app), et fait double emploi
  avec le compteur physique de l'imprimante elle-même. Le porteur du
  projet a demandé une "rupture plus profonde" — d'où le pivot vers la
  traçabilité des fichiers reçus plutôt que le comptage de pages.
- **Verrouillage SNMP du comptage de stock réel** : jamais retenu (déjà
  écarté au tour précédent, cf. README) — imprimante réseau physique
  nécessaire pour tester, non disponible ici.
