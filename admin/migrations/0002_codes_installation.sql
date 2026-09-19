-- Trace de chaque installation autorisée : un code n'est fabriqué que
-- lorsque le porteur du projet a été prévenu d'une installation précise
-- (identifiant machine communiqué à l'avance, souvent par un agent ou un
-- maintenancier sur le terrain). Sert d'historique consultable, distinct
-- des licences (qui, elles, portent une durée d'abonnement).
CREATE TABLE IF NOT EXISTS codes_installation (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  machine_id TEXT NOT NULL,
  code TEXT NOT NULL,
  note TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
