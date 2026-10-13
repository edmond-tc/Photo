-- Fiche publique de chaque boutique inscrite au relais (adresse, horaires,
-- téléphone, plan, monnaie) et numéros qu'elle a bloqués. Table à part :
-- les migrations sont rejouées à chaque déploiement, et « CREATE TABLE IF
-- NOT EXISTS » se rejoue sans risque (un ALTER TABLE, non).
CREATE TABLE IF NOT EXISTS relais_fiches (
  code     TEXT PRIMARY KEY,
  fiche    TEXT NOT NULL DEFAULT '{}',
  bloques  TEXT NOT NULL DEFAULT '[]'
);
