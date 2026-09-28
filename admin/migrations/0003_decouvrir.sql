-- Espace « Découvrir » de l'application Envoyeur Kiosque : les projets du
-- porteur du projet et les publicités de partenaires. Visible seulement
-- quand le client touche « Découvrir » (jamais pendant un envoi).
CREATE TABLE IF NOT EXISTS decouvrir (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  titre       TEXT NOT NULL,
  texte       TEXT,
  image_url   TEXT,
  lien        TEXT,
  bouton      TEXT,
  genre       TEXT NOT NULL DEFAULT 'projet', -- 'projet' | 'partenaire'
  ordre       INTEGER NOT NULL DEFAULT 0,
  actif       INTEGER NOT NULL DEFAULT 1,
  created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
