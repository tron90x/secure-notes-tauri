use crate::models::{Database, Profile};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::HashMap;

/// Métadonnées uniquement : ni contenu, ni pièce jointe, ni payload chiffré dans la palette.
#[derive(Debug, Serialize)]
pub struct NoteMatch {
    pub id: String,
    pub title: String,
    pub profile_id: String,
    pub profile_name: String,
    pub is_locked: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CommandMatches {
    pub notes: Vec<NoteMatch>,
    pub profiles: Vec<Profile>,
    pub tags: Vec<(String, usize)>,
    pub folders: Vec<FolderMatch>,
}

#[derive(Debug, Serialize)]
pub struct FolderMatch { pub id: String, pub name: String }

pub fn search(db: &Database, query: &str) -> CommandMatches {
    let query: String = query.trim().chars().take(200).collect::<String>().to_lowercase();
    let words: Vec<_> = query.split_whitespace().collect();
    let matches = |value: &str| {
        let lower = value.to_lowercase();
        words.iter().all(|word| lower.contains(word))
    };
    let mut notes: Vec<_> = db.notes.iter().filter(|n| n.trashed_at.is_none())
        .filter(|n| if n.is_locked { words.is_empty() } else { matches(&format!("{} {}", n.title, n.content)) })
        .map(|n| {
            let title = n.title.to_lowercase();
            let rank = if query.is_empty() { 0 } else if title.starts_with(&query) { 3 } else if title.contains(&query) { 2 } else { 1 };
            let updated_at = n.modified_history.iter().copied().max().unwrap_or(n.created_at).max(n.created_at);
            (rank, NoteMatch {
                id: n.id.clone(), title: if n.is_locked { String::new() } else { n.title.clone() },
                profile_id: n.profile_id.clone(),
                profile_name: db.profiles.iter().find(|p| p.id == n.profile_id).map(|p| p.name.clone()).unwrap_or_default(),
                is_locked: n.is_locked, updated_at,
            })
        }).collect();
    notes.sort_by(|(ar, a), (br, b)| br.cmp(ar).then(b.updated_at.cmp(&a.updated_at)).then(a.id.cmp(&b.id)));
    let mut profiles: Vec<_> = db.profiles.iter().filter(|p| matches(&p.name)).cloned().collect();
    profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    profiles.truncate(8);
    let mut counts = HashMap::new();
    for note in db.notes.iter().filter(|n| n.trashed_at.is_none() && !n.is_locked) {
        for tag in &note.tags {
            if matches(tag) { *counts.entry(tag.clone()).or_insert(0usize) += 1; }
        }
    }
    let mut tags: Vec<_> = counts.into_iter().collect();
    tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    tags.truncate(8);
    let folders = db.smart_folders.iter().filter(|f| matches(&f.name)).take(8)
        .map(|f| FolderMatch { id: f.id.clone(), name: f.name.clone() }).collect();
    CommandMatches { notes: notes.into_iter().take(12).map(|(_, note)| note).collect(), profiles, tags, folders }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::sample_note;

    #[test]
    fn metadata_search_never_discloses_locked_content_or_attachments() {
        let mut db = Database::default();
        let free = sample_note();
        let mut locked = free.clone();
        locked.id = "locked".into();
        locked.is_locked = true;
        locked.title = "titre individuel confidentiel".into();
        locked.content = "contenu individuel confidentiel".into();
        locked.tags = vec!["tagsecret".into()];
        locked.locked_payload = Some("payload-secret".into());
        db.notes = vec![free, locked];
        let result = search(&db, "");
        assert_eq!(result.notes.len(), 2);
        assert!(result.notes.iter().find(|n| n.is_locked).unwrap().title.is_empty());
        let json = serde_json::to_string(&result).unwrap();
        for secret in ["confidentiel", "payload-secret", "c2VjcmV0", "secret.txt", "tagsecret", "Contenu secret"] { assert!(!json.contains(secret)); }
        assert!(search(&db, "confidentiel").notes.is_empty());
    }

    #[test]
    fn search_matches_all_words_and_ranks_titles_before_content() {
        let mut db = Database::default();
        let mut body_match = sample_note();
        body_match.id = "body".into();
        body_match.title = "Autre note".into();
        body_match.content = "PROJET budget annuel".into();
        let mut title_match = body_match.clone();
        title_match.id = "title".into();
        title_match.title = "Projet budget".into();
        title_match.content.clear();
        db.notes = vec![body_match, title_match];
        let result = search(&db, "  projet BUDGET ");
        assert_eq!(result.notes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), ["title", "body"]);
        assert!(search(&db, "projet absent").notes.is_empty());
    }

    #[test]
    fn recent_results_are_bounded_and_trash_is_excluded() {
        let mut db = Database::default();
        for i in 0..20 {
            let mut note = sample_note();
            note.id = format!("n{i:02}");
            note.modified_history.push(note.created_at + chrono::Duration::minutes(i));
            db.notes.push(note);
        }
        db.notes[19].trashed_at = Some(Utc::now());
        let result = search(&db, "");
        assert_eq!(result.notes.len(), 12);
        assert_eq!(result.notes[0].id, "n18");
        assert!(!result.notes.iter().any(|n| n.id == "n19"));
    }

    #[test]
    fn profiles_and_tags_are_filtered_without_counting_hidden_notes() {
        let mut db = Database::default();
        db.profiles.push(Profile { id: "p".into(), name: "Travail".into(), created_at: Utc::now() });
        let mut note = sample_note();
        note.tags = vec!["travail".into()];
        let mut hidden = note.clone();
        hidden.is_locked = true;
        db.notes = vec![note, hidden];
        let result = search(&db, "TRAV");
        assert_eq!(result.profiles.len(), 1);
        assert_eq!(result.tags, [("travail".into(), 1)]);
    }

    #[test]
    fn folder_palette_entries_do_not_include_saved_queries() {
        let mut db = Database::default();
        db.smart_folders.push(crate::models::SmartFolder { id: "f".into(), name: "Urgent".into(),
            params: crate::models::SearchParams { query: Some("critère confidentiel".into()), ..Default::default() },
            created_at: Utc::now(), updated_at: Utc::now() });
        let result = search(&db, "urgent");
        assert_eq!(result.folders.len(), 1);
        assert!(!serde_json::to_string(&result).unwrap().contains("confidentiel"));
    }
}
