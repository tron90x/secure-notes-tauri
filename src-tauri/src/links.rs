use crate::models::*;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct Reference { pub target: String, pub label: String }

pub fn references(text: &str) -> Vec<Reference> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]" ) else { break; };
        let raw = rest[..end].trim();
        let (target, label) = raw.split_once('|').unwrap_or((raw, raw));
        let (target, label) = (target.trim(), label.trim());
        if !target.is_empty() && target.chars().count() <= 80 && raw.chars().count() <= 200 {
            out.push(Reference { target: target.into(), label: if label.is_empty() { target.into() } else { label.into() } });
        }
        rest = &rest[end + 2..];
    }
    out
}

pub enum Resolution<'a> { Resolved(&'a Note), Locked(&'a Note), Trashed(&'a Note), Missing, Ambiguous(Vec<&'a Note>) }

pub struct Index<'a> {
    by_id: HashMap<&'a str, &'a Note>,
    by_title: HashMap<String, Vec<&'a Note>>,
}

impl<'a> Index<'a> {
    pub fn new(db: &'a Database) -> Self {
        let mut by_id = HashMap::new();
        let mut by_title: HashMap<String, Vec<&Note>> = HashMap::new();
        for note in &db.notes {
            by_id.insert(note.id.as_str(), note);
            if !note.is_locked && note.trashed_at.is_none() { by_title.entry(note.title.to_lowercase()).or_default().push(note); }
        }
        Self { by_id, by_title }
    }

    pub fn resolve(&self, reference: &str) -> Resolution<'a> {
        let normalized = uuid::Uuid::parse_str(reference).ok().map(|id| id.to_string());
        if let Some(note) = self.by_id.get(reference).or_else(|| normalized.as_deref().and_then(|id| self.by_id.get(id))) {
            if note.trashed_at.is_some() { return Resolution::Trashed(note); }
            return if note.is_locked { Resolution::Locked(note) } else { Resolution::Resolved(note) };
        }
        if normalized.is_some() { return Resolution::Missing; }
        match self.by_title.get(&reference.trim().to_lowercase()) {
            Some(notes) if notes.len() == 1 => Resolution::Resolved(notes[0]),
            Some(notes) => Resolution::Ambiguous(notes.clone()),
            None => Resolution::Missing,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkTarget {
    pub id: String, pub title: String, pub profile_id: String, pub profile_name: String,
    pub is_locked: bool, pub preview: String,
}

fn summary(db: &Database, note: &Note) -> LinkTarget {
    LinkTarget { id: note.id.clone(), title: if note.is_locked { String::new() } else { note.title.clone() },
        profile_id: note.profile_id.clone(), profile_name: db.profiles.iter().find(|p| p.id == note.profile_id).map(|p| p.name.clone()).unwrap_or_default(),
        is_locked: note.is_locked, preview: if note.is_locked { String::new() } else { note.content.chars().take(240).collect() } }
}

#[derive(Debug, Serialize)]
pub struct ResolvedReference {
    pub reference: String, pub label: String, pub status: String, pub targets: Vec<LinkTarget>, pub can_create: bool,
}

fn resolved(db: &Database, index: &Index<'_>, reference: &Reference) -> ResolvedReference {
    let (status, targets) = match index.resolve(&reference.target) {
        Resolution::Resolved(note) => ("resolved", vec![summary(db, note)]),
        Resolution::Locked(note) => ("locked", vec![summary(db, note)]),
        Resolution::Trashed(note) => ("trashed", vec![summary(db, note)]),
        Resolution::Ambiguous(notes) => ("ambiguous", notes.into_iter().take(8).map(|n| summary(db, n)).collect()),
        Resolution::Missing => ("missing", vec![]),
    };
    ResolvedReference { reference: reference.target.clone(), label: reference.label.clone(), status: status.into(), targets,
        can_create: status == "missing" && uuid::Uuid::parse_str(&reference.target).is_err() }
}

pub fn resolve(db: &Database, target: &str) -> Result<ResolvedReference, String> {
    if target.trim().is_empty() || target.chars().count() > 80 { return Err("Référence invalide".into()); }
    Ok(resolved(db, &Index::new(db), &Reference { target: target.trim().into(), label: target.trim().into() }))
}

#[derive(Debug, Serialize)]
pub struct NoteContext {
    pub incoming: Vec<LinkTarget>, pub outgoing: Vec<ResolvedReference>, pub incoming_total: usize, pub outgoing_total: usize,
}

pub fn context(db: &Database, id: &str) -> Result<NoteContext, String> {
    let note = db.notes.iter().find(|n| n.id == id && n.trashed_at.is_none()).ok_or("Note introuvable")?;
    if note.is_locked { return Err("Index des liens indisponible pour une note protégée".into()); }
    let index = Index::new(db);
    let mut seen = HashSet::new();
    let outgoing: Vec<_> = references(&note.content).into_iter().filter(|r| seen.insert(r.target.clone()))
        .map(|r| resolved(db, &index, &r)).collect();
    let mut incoming: Vec<_> = db.notes.iter().filter(|n| n.id != id && n.trashed_at.is_none() && !n.is_locked)
        .filter(|n| references(&n.content).iter().any(|r| matches!(index.resolve(&r.target), Resolution::Resolved(target) if target.id == id)))
        .map(|n| summary(db, n)).collect();
    incoming.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.id.cmp(&b.id)));
    let (incoming_total, outgoing_total) = (incoming.len(), outgoing.len());
    Ok(NoteContext { incoming: incoming.into_iter().take(50).collect(), outgoing: outgoing.into_iter().take(50).collect(), incoming_total, outgoing_total })
}

pub fn create_target(s: &mut crate::Session, source_id: &str, reference: &str) -> Result<String, String> {
    let source = s.db.notes.iter().find(|n| n.id == source_id && n.trashed_at.is_none() && !n.is_locked).ok_or("Note source introuvable ou protégée")?;
    if !references(&source.content).iter().any(|r| r.target == reference) { return Err("Cette référence n'appartient plus à la note".into()); }
    match Index::new(&s.db).resolve(reference) {
        Resolution::Resolved(note) => return Ok(note.id.clone()),
        Resolution::Missing if uuid::Uuid::parse_str(reference).is_err() => (),
        _ => return Err("Choisissez une cible existante ou restaurez la note supprimée".into()),
    }
    let profile_id = source.profile_id.clone();
    crate::mutate_database(s, |db| {
        if !db.profiles.iter().any(|p| p.id == profile_id) { return Err("Profil source introuvable".into()); }
        let id = uuid::Uuid::new_v4().to_string();
        db.notes.push(Note { id: id.clone(), profile_id, revision: 0, title: reference.into(), content: String::new(),
            is_locked: false, locked_nonce: None, locked_payload: None, locked_salt: None, locked_format: 0,
            created_at: chrono::Utc::now(), modified_history: vec![], attachment_name: None, attachment_data: None,
            wallpaper: "aurora".into(), glass: true, tags: vec![], highlights: vec![], versions: vec![], trashed_at: None });
        Ok(id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{sample_note, Sandbox};

    #[test]
    fn references_support_titles_stable_ids_aliases_and_unicode() {
        let r = references("[[Été]] [[id-1|Mon libellé]] [[  ]] [[non fermé");
        assert_eq!(r.len(), 2); assert_eq!(r[1].target, "id-1"); assert_eq!(r[1].label, "Mon libellé");
    }

    #[test]
    fn stable_links_survive_rename_and_duplicate_titles_are_explicit() {
        let mut db = Database::default();
        let mut first = sample_note(); first.id = "target".into(); first.title = "Cible".into();
        let mut second = first.clone(); second.id = "duplicate".into();
        db.notes = vec![first, second];
        assert_eq!(resolve(&db, "Cible").unwrap().status, "ambiguous");
        db.notes[0].title = "Nouveau nom".into();
        assert_eq!(resolve(&db, "target").unwrap().targets[0].title, "Nouveau nom");
    }

    #[test]
    fn contexts_exclude_private_sources_and_mask_private_targets() {
        let mut db = Database::default();
        let mut target = sample_note(); target.id = "target".into(); target.content = "[[locked]] [[Absent]]".into();
        let mut free = sample_note(); free.content = "[[target|Cible]] [[target]]".into();
        let mut locked = free.clone(); locked.id = "locked".into(); locked.is_locked = true; locked.title = "titre privé".into();
        db.notes = vec![target, free, locked];
        let c = context(&db, "target").unwrap();
        assert_eq!(c.incoming_total, 1); assert_eq!(c.outgoing_total, 2);
        assert_eq!(c.outgoing[0].status, "locked");
        assert!(c.outgoing[0].targets[0].title.is_empty()); assert!(c.outgoing[0].targets[0].preview.is_empty());
        assert!(!serde_json::to_string(&c).unwrap().contains("titre privé"));
        assert!(context(&db, "locked").is_err());
    }

    #[test]
    fn creation_from_missing_reference_is_atomic_and_idempotent() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session();
        crate::mutate_database(&mut s, |db| {
            db.profiles.push(Profile { id: "p".into(), name: "Perso".into(), created_at: chrono::Utc::now() });
            let mut n = sample_note(); n.content = "[[Nouvelle cible]]".into(); db.notes.push(n); Ok(())
        }).unwrap();
        let id = create_target(&mut s, "n", "Nouvelle cible").unwrap();
        assert_eq!(create_target(&mut s, "n", "Nouvelle cible").unwrap(), id);
        assert_eq!(s.db.notes.len(), 2);
        assert!(create_target(&mut s, "n", "Autre cible").is_err());
    }

    #[test]
    fn deleted_stable_ids_never_fall_back_to_an_unrelated_title_and_contexts_are_bounded() {
        let id = uuid::Uuid::new_v4().to_string();
        let mut db = Database::default();
        let mut target = sample_note(); target.id = "target".into(); target.title = id.clone(); target.content.clear();
        db.notes.push(target);
        assert_eq!(resolve(&db, &id).unwrap().status, "missing");
        for i in 0..55 {
            let mut note = sample_note(); note.id = format!("source-{i}"); note.content = "[[target]]".into(); db.notes.push(note);
        }
        let result = context(&db, "target").unwrap();
        assert_eq!(result.incoming_total, 55); assert_eq!(result.incoming.len(), 50);
    }
}
