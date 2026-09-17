use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultFile {
    pub version: u32,
    pub name: String,
    pub salt: String,       // base64
    pub nonce: String,      // base64
    pub ciphertext: String, // base64
    /// Coffre leurre (duress) : seconde base plausible sous un 2e mot de passe.
    #[serde(default)]
    pub decoy_salt: Option<String>,
    #[serde(default)]
    pub decoy_nonce: Option<String>,
    #[serde(default)]
    pub decoy_ciphertext: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Database {
    pub profiles: Vec<Profile>,
    pub notes: Vec<Note>,
    /// Objectif de mots par jour (0 = désactivé).
    #[serde(default)]
    pub daily_goal_words: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Highlight {
    pub id: String,
    pub start: usize,
    pub end: usize,
    pub color: String, // ex: yellow, green, pink, blue, orange
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteVersion {
    pub title: String,
    pub content: String,
    pub saved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub profile_id: String,
    pub title: String,
    /// Si locked = false : content en clair (mais DB globale chiffrée).
    /// Si locked = true : content = "" et payload_chiffre contient le contenu chiffré avec note_password.
    pub content: String,
    pub is_locked: bool,
    pub locked_nonce: Option<String>,
    pub locked_payload: Option<String>,
    pub locked_salt: Option<String>,
    pub created_at: DateTime<Utc>,
    pub modified_history: Vec<DateTime<Utc>>,
    pub attachment_name: Option<String>,
    pub attachment_data: Option<String>, // base64 (chiffré via DB globale, donc ok)
    pub wallpaper: String, // clé : aurora, sunset, ocean, neon, carbon, sakura...
    pub glass: bool,
    #[serde(default)]
    pub highlights: Vec<Highlight>,
    /// Historique du contenu (plafonné à 20), pour restaurer une version.
    #[serde(default)]
    pub versions: Vec<NoteVersion>,
    /// Soft-delete : Some(date) = dans la corbeille.
    #[serde(default)]
    pub trashed_at: Option<DateTime<Utc>>,
    /// Étiquettes.normalisées en minuscules côté backend.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: Option<String>,
    pub profile_id: Option<String>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    #[serde(default)]
    pub tag: Option<String>,
}

// ---------- Dashboard ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DayStat {
    pub date: String, // JJ/MM
    pub creations: usize,
    pub modifications: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatDay {
    pub date: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileStat {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopNote {
    pub title: String,
    pub profile: String,
    pub words: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub profiles_count: usize,
    pub notes_total: usize,
    pub trash_count: usize,
    pub locked_count: usize,
    pub unlocked_count: usize,
    pub attachments_count: usize,
    pub attachments_bytes: u64,
    pub highlights_total: usize,
    pub highlights_by_color: Vec<(String, usize)>,
    pub versions_total: usize,
    pub words_total: usize,
    pub avg_words: usize,
    pub glass_count: usize,
    pub wallpapers: Vec<(String, usize)>,
    pub per_profile: Vec<ProfileStat>,
    pub last_30d: Vec<DayStat>,
    pub heat_84d: Vec<HeatDay>,
    pub longest: Vec<TopNote>,
    pub vault_bytes: u64,
    pub vault_path: String,
    pub oldest: Option<DateTime<Utc>>,
    pub newest: Option<DateTime<Utc>>,
    pub auto_lock_secs: u64,
    // Streak & objectif
    pub streak_current: usize,
    pub streak_best: usize,
    pub days_active: usize,
    pub words_today: usize,
    pub daily_goal_words: usize,
    // Tags & leurre
    pub top_tags: Vec<(String, usize)>,
    pub decoy: bool,
    pub has_decoy: bool,
}

// ---------- Graphe multi-couches : notes [[...]] + tags + @mentions ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub profile: String,
    pub links: usize,
    /// "note" | "tag" | "person" | "place" | "idea" | "date" (+ "mention" legacy = person).
    #[serde(default = "default_graph_kind_note")]
    pub kind: String,
}

fn default_graph_kind_note() -> String {
    "note".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    /// "link" | "tag" | "person" | "place" | "idea" | "date" | "cooc" (+ "mention" legacy = person)
    #[serde(default = "default_graph_edge_link")]
    pub kind: String,
    #[serde(default = "default_graph_weight")]
    pub weight: usize,
}

fn default_graph_edge_link() -> String {
    "link".into()
}

fn default_graph_weight() -> usize {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

// ---------- Backups ----------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub name: String,
    pub bytes: u64,
    pub modified: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Les bases créées avant les champs highlights/versions/trashed_at doivent rester lisibles.
    #[test]
    fn old_note_json_stays_compatible() {
        let json = r#"{
            "id": "n1", "profile_id": "p1", "title": "T", "content": "C",
            "is_locked": false, "locked_nonce": null, "locked_payload": null, "locked_salt": null,
            "created_at": "2026-01-01T00:00:00Z", "modified_history": [],
            "attachment_name": null, "attachment_data": null,
            "wallpaper": "aurora", "glass": true
        }"#;
        let n: Note = serde_json::from_str(json).unwrap();
        assert!(n.highlights.is_empty());
        assert!(n.versions.is_empty());
        assert!(n.trashed_at.is_none());
    }
}
