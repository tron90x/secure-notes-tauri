use crate::models::*;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;

#[derive(Debug)]
enum Rule {
    Text(String), Tag(String), Profile(String), After(DateTime<Utc>), Before(DateTime<Utc>),
    Modified(DateTime<Utc>), Attachment, Untagged, Locked(bool),
}

pub struct CompiledSearch {
    params: SearchParams,
    rules: Vec<(bool, Rule)>,
    needs_plaintext: bool,
}

fn tokens(query: &str) -> Result<Vec<(bool, String)>, String> {
    if query.chars().count() > 1000 { return Err("Requête trop longue (maximum 1000 caractères)".into()); }
    let mut result = vec![];
    let mut value = String::new();
    let (mut quoted, mut escaped, mut literal) = (false, false, false);
    for c in query.chars() {
        if escaped { if c != '"' && c != '\\' { value.push('\\'); } value.push(c); escaped = false; continue; }
        if quoted && c == '\\' { escaped = true; continue; }
        if c == '"' {
            if value.is_empty() && !quoted { literal = true; }
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            if !value.is_empty() { result.push((literal, std::mem::take(&mut value))); }
            literal = false;
        } else { value.push(c); }
    }
    if quoted || escaped { return Err("Guillemets non fermés dans la recherche".into()); }
    if !value.is_empty() { result.push((literal, value)); }
    Ok(result)
}

fn date(value: &str) -> Result<DateTime<Utc>, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|d| d.and_utc()).ok_or_else(|| "Date de recherche invalide (format YYYY-MM-DD)".into())
}

impl CompiledSearch {
    pub fn new(params: &SearchParams, now: DateTime<Utc>) -> Result<Self, String> {
        if let (Some(from), Some(to)) = (params.date_from, params.date_to) {
            if from >= to { return Err("La date de fin doit suivre la date de début".into()); }
        }
        let mut rules = vec![];
        let mut needs_plaintext = params.tag.as_ref().is_some_and(|t| !t.is_empty());
        for (literal, token) in tokens(params.query.as_deref().unwrap_or_default())? {
            let negative = !literal && token.starts_with('-') && token.chars().count() > 1;
            let value = if negative { &token[1..] } else { &token }.to_lowercase();
            let (operator, argument) = if literal { ("", value.as_str()) } else { value.split_once(':').unwrap_or(("", &value)) };
            let rule = match operator {
                "tag" => { if argument.is_empty() { return Err("Un tag est requis après tag:".into()); } Rule::Tag(argument.into()) },
                "profil" | "profile" => { if argument.is_empty() { return Err("Un profil est requis après profil:".into()); } Rule::Profile(argument.into()) },
                "apres" | "après" | "after" => Rule::After(date(argument)?),
                "avant" | "before" => Rule::Before(date(argument)?),
                "modifiee" | "modifiée" | "modifie" | "modified" => {
                    let days = argument.strip_suffix('j').or_else(|| argument.strip_suffix('d')).unwrap_or(argument)
                        .parse::<i64>().ok().filter(|d| (1..=3650).contains(d)).ok_or("Durée invalide : utilisez modifiee:7j (1 à 3650 jours)")?;
                    Rule::Modified(now - Duration::days(days))
                }
                "avec" | "with" => match argument { "piece-jointe" | "pièce-jointe" | "attachment" => Rule::Attachment, _ => return Err("Filtre avec: inconnu (utilisez avec:piece-jointe)".into()) },
                "sans" | "without" => match argument { "tag" | "tags" => Rule::Untagged, _ => return Err("Filtre sans: inconnu (utilisez sans:tag)".into()) },
                "statut" | "is" => match argument {
                    "verrouillee" | "verrouillée" | "locked" => Rule::Locked(true),
                    "libre" | "unlocked" => Rule::Locked(false),
                    _ => return Err("Statut inconnu (libre ou verrouillee)".into()),
                },
                _ => Rule::Text(value),
            };
            needs_plaintext |= matches!(rule, Rule::Text(_) | Rule::Tag(_) | Rule::Attachment | Rule::Untagged);
            rules.push((negative, rule));
        }
        Ok(Self { params: params.clone(), rules, needs_plaintext })
    }

    pub fn matches(&self, note: &Note, profiles: &[Profile]) -> bool {
        if note.trashed_at.is_some() || (self.needs_plaintext && note.is_locked) { return false; }
        if self.params.profile_id.as_ref().is_some_and(|id| !id.is_empty() && id != &note.profile_id) { return false; }
        if self.params.tag.as_ref().is_some_and(|tag| !tag.is_empty() && !note.tags.iter().any(|t| t.to_lowercase() == tag.to_lowercase())) { return false; }
        if self.params.date_from.is_some_and(|d| note.created_at < d) || self.params.date_to.is_some_and(|d| note.created_at >= d) { return false; }
        self.rules.iter().all(|(negative, rule)| {
            let matched = match rule {
                Rule::Text(text) => note.title.to_lowercase().contains(text) || note.content.to_lowercase().contains(text),
                Rule::Tag(tag) => note.tags.iter().any(|t| t.to_lowercase() == *tag),
                Rule::Profile(value) => note.profile_id.to_lowercase() == *value || profiles.iter().any(|p| p.id == note.profile_id && p.name.to_lowercase() == *value),
                Rule::After(d) => note.created_at >= *d,
                Rule::Before(d) => note.created_at < *d,
                Rule::Modified(d) => note.modified_history.iter().copied().max().unwrap_or(note.created_at).max(note.created_at) >= *d,
                Rule::Attachment => note.attachment_name.is_some(),
                Rule::Untagged => note.tags.is_empty(),
                Rule::Locked(locked) => note.is_locked == *locked,
            };
            matched != *negative
        })
    }
}

#[derive(Serialize)]
pub struct FolderSummary {
    #[serde(flatten)]
    pub folder: SmartFolder,
    pub count: Option<usize>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct FolderResults {
    pub folder: SmartFolder,
    pub notes: Vec<Note>,
}

pub fn list_folders(db: &Database, now: DateTime<Utc>) -> Vec<FolderSummary> {
    db.smart_folders.iter().map(|folder| {
        let result = CompiledSearch::new(&folder.params, now);
        let (count, error) = match result {
            Ok(filter) => (Some(db.notes.iter().filter(|n| filter.matches(n, &db.profiles)).count()), None),
            Err(error) => (None, Some(error)),
        };
        FolderSummary { folder: folder.clone(), count, error }
    }).collect()
}

pub fn save_folder(db: &mut Database, id: Option<String>, name: String, params: SearchParams, now: DateTime<Utc>) -> Result<SmartFolder, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 64 { return Err("Nom de dossier requis (1 à 64 caractères)".into()); }
    CompiledSearch::new(&params, now)?;
    if db.smart_folders.iter().any(|f| f.name.to_lowercase() == name.to_lowercase() && Some(&f.id) != id.as_ref()) { return Err("Un dossier porte déjà ce nom".into()); }
    if let Some(id) = id {
        let folder = db.smart_folders.iter_mut().find(|f| f.id == id).ok_or("Dossier introuvable")?;
        folder.name = name; folder.params = params; folder.updated_at = now;
        Ok(folder.clone())
    } else {
        if db.smart_folders.len() >= 50 { return Err("Maximum 50 dossiers intelligents".into()); }
        let folder = SmartFolder { id: uuid::Uuid::new_v4().to_string(), name, params, created_at: now, updated_at: now };
        db.smart_folders.push(folder.clone());
        Ok(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::sample_note;

    #[test]
    fn phrases_negations_and_structured_filters_combine() {
        let mut note = sample_note();
        note.content = "réunion équipe budget validé".into();
        note.tags = vec!["travail".into()];
        let profiles = [Profile { id: "p".into(), name: "Mon Travail".into(), created_at: Utc::now() }];
        let p = SearchParams { query: Some(r#""réunion équipe" -annulée tag:travail profil:"Mon Travail" avec:piece-jointe"#.into()), ..SearchParams::default() };
        assert!(CompiledSearch::new(&p, Utc::now()).unwrap().matches(&note, &profiles));
        note.content.push_str(" annulée");
        assert!(!CompiledSearch::new(&p, Utc::now()).unwrap().matches(&note, &profiles));
    }

    #[test]
    fn protected_content_cannot_be_inferred_through_negative_or_attachment_filters() {
        let mut note = sample_note(); note.is_locked = true;
        for query in ["secret", "-secret", "sans:tag", "-tag:secret", "avec:piece-jointe"] {
            let p = SearchParams { query: Some(query.into()), ..SearchParams::default() };
            assert!(!CompiledSearch::new(&p, Utc::now()).unwrap().matches(&note, &[]));
        }
        let p = SearchParams { query: Some("statut:verrouillee profil:p".into()), ..SearchParams::default() };
        assert!(CompiledSearch::new(&p, Utc::now()).unwrap().matches(&note, &[]));
    }

    #[test]
    fn invalid_queries_are_rejected_and_urls_remain_literal() {
        for query in ["\"non fermé", "apres:2026-02-31", "modifiee:0j", "avec:inconnu", "tag:"] {
            assert!(CompiledSearch::new(&SearchParams { query: Some(query.into()), ..SearchParams::default() }, Utc::now()).is_err());
        }
        let mut note = sample_note(); note.content = "https://example.test".into();
        let filter = CompiledSearch::new(&SearchParams { query: Some("https://example.test".into()), ..SearchParams::default() }, Utc::now()).unwrap();
        assert!(filter.matches(&note, &[]));
    }

    #[test]
    fn saved_folders_recompute_relative_dates_and_follow_database_changes() {
        let now = date("2026-10-25").unwrap();
        let mut db = Database::default();
        let mut note = sample_note(); note.created_at = now - Duration::days(1); db.notes.push(note);
        let folder = save_folder(&mut db, None, "Récentes".into(), SearchParams { query: Some("modifiee:7j".into()), ..SearchParams::default() }, now).unwrap();
        assert_eq!(list_folders(&db, now)[0].count, Some(1));
        assert_eq!(list_folders(&db, now + Duration::days(8))[0].count, Some(0));
        db.notes[0].modified_history.push(now + Duration::days(8));
        assert_eq!(list_folders(&db, now + Duration::days(8))[0].count, Some(1));
        assert!(save_folder(&mut db, None, "récentes".into(), SearchParams::default(), now).is_err());
        save_folder(&mut db, Some(folder.id), "Sans tags".into(), SearchParams { query: Some("sans:tag".into()), ..SearchParams::default() }, now).unwrap();
        assert_eq!(db.notes.len(), 1);
        let loaded: Database = serde_json::from_str(&serde_json::to_string(&db).unwrap()).unwrap();
        assert_eq!(loaded.smart_folders[0].name, "Sans tags");
    }
}
