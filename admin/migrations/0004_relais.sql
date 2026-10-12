-- Relais Internet (voir admin/site/relais.js) : les boutiques qui reçoivent
-- aussi par Internet, et les commandes en route. Aucun fichier ici : ils
-- sont dans le stockage, sous `relais/`, et effacés dès la récupération.
CREATE TABLE IF NOT EXISTS relais_boutiques (
  code             TEXT PRIMARY KEY,
  empreinte_secret TEXT NOT NULL,
  nom              TEXT,
  tarifs           TEXT,
  cree             TEXT NOT NULL,
  vu               TEXT,
  actif            INTEGER NOT NULL DEFAULT 1,
  octets_attente   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS relais_commandes (
  id        TEXT PRIMARY KEY,
  code      TEXT NOT NULL,
  jeton     TEXT NOT NULL UNIQUE,
  champs    TEXT NOT NULL,
  fichiers  TEXT NOT NULL,
  etape     TEXT NOT NULL DEFAULT 'en_ligne',
  numero    TEXT,
  paye      INTEGER NOT NULL DEFAULT 0,
  message   TEXT,
  cree      TEXT NOT NULL,
  recupere  TEXT
);

CREATE INDEX IF NOT EXISTS relais_commandes_attente ON relais_commandes (code, etape, cree);
