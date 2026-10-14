-- Amphi en ligne (voir admin/site/amphi.js) : les étudiants publient depuis
-- Internet (lien partagé dans le groupe Facebook ou WhatsApp) ; le téléphone
-- du portail de l'amphi vient chercher et affiche sur le Wi-Fi de l'amphi.
-- Les fichiers sont dans le stockage, sous `amphi/`, effacés dès récupération.
CREATE TABLE IF NOT EXISTS amphi_comptes (
  code             TEXT PRIMARY KEY,
  empreinte_secret TEXT NOT NULL,
  titre            TEXT,
  cree             TEXT NOT NULL,
  vu               TEXT
);

CREATE TABLE IF NOT EXISTS amphi_envois (
  id      TEXT PRIMARY KEY,
  code    TEXT NOT NULL,
  sorte   TEXT NOT NULL,
  champs  TEXT NOT NULL,
  fichier TEXT,
  nom     TEXT,
  type    TEXT,
  taille  INTEGER NOT NULL DEFAULT 0,
  ip      TEXT,
  cree    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS amphi_envois_code ON amphi_envois (code, cree);
