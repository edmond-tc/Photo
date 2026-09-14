#[derive(Clone, serde::Serialize)]
pub struct QueueItem {
    pub id: i64,
    pub original_name: String,
    pub path: String,
    pub client_name: Option<String>,
    pub client_telephone: Option<String>,
    pub source: String,
    pub kind: String,
    pub status: String,
    pub received_at: String,
    pub taille_octets: i64,
    pub protege: bool,
    pub format_detecte: Option<String>,
    pub copies: i64,
    pub couleur: bool,
    pub format_papier: String,
    pub plage_pages: Option<String>,
    pub finitions: Vec<String>,
    pub prix: Option<i64>,
    pub employe: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Tarif {
    pub id: i64,
    pub service: String,
    pub libelle: String,
    pub prix_unitaire: i64,
    pub unite: String,
}

#[derive(Clone, serde::Serialize)]
pub struct Transaction {
    pub id: i64,
    pub file_queue_id: Option<i64>,
    pub description: String,
    pub montant: i64,
    pub moyen_paiement: String,
    pub statut: String,
    pub employe: Option<String>,
    pub created_at: String,
}

#[derive(Clone, serde::Serialize)]
pub struct Depense {
    pub id: i64,
    pub description: String,
    pub montant: i64,
    pub categorie: String,
    pub created_at: String,
}

#[derive(Clone, serde::Serialize)]
pub struct StockItem {
    pub item: String,
    pub libelle: String,
    pub quantite: f64,
    pub seuil_alerte: f64,
    pub unite: String,
    pub alerte: bool,
}

#[derive(Clone, serde::Serialize)]
pub struct Employe {
    pub id: i64,
    pub nom: String,
    pub actif: bool,
}
