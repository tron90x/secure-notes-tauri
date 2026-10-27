use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct EditorContent {
    pub title: String,
    pub content: String,
    pub attachment_name: Option<String>,
    pub attachment_data: Option<String>,
    pub wallpaper: String,
    pub glass: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Default for EditorContent {
    fn default() -> Self {
        Self { title: String::new(), content: String::new(), attachment_name: None, attachment_data: None, wallpaper: "aurora".into(), glass: true, tags: vec![] }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct EditorHistory {
    original: Option<NoteVersion>,
    versions: Vec<NoteVersion>,
    highlights: Vec<Highlight>,
    checkpointed: bool,
    last_word_count: usize,
}

#[derive(Serialize, Deserialize)]
struct DraftPayload {
    content: EditorContent,
    history: EditorHistory,
}

#[derive(Clone)]
pub struct EditorSession {
    draft_id: String,
    note_id: Option<String>,
    profile_id: String,
    revision: u64,
    key: Option<Zeroizing<[u8; 32]>>,
    salt: Option<Vec<u8>>,
    history: EditorHistory,
}

impl Drop for EditorSession {
    fn drop(&mut self) {
        for version in &mut self.history.versions {
            version.title.zeroize();
            version.content.zeroize();
        }
        if let Some(original) = &mut self.history.original {
            original.title.zeroize();
            original.content.zeroize();
        }
        // Les clés individuelles sont effacées par Zeroizing.
    }
}

#[derive(Default, Deserialize)]
pub struct OpenEditorParams {
    pub note_id: Option<String>,
    pub draft_id: Option<String>,
    pub profile_id: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub recover_as_new: bool,
}

#[derive(Serialize)]
pub struct EditorView {
    pub token: String,
    pub draft_id: String,
    pub note_id: Option<String>,
    pub profile_id: String,
    pub protected: bool,
    pub content: EditorContent,
}

#[derive(Deserialize)]
pub struct SaveEditorParams {
    pub token: String,
    pub content: EditorContent,
    pub profile_id: Option<String>,
    pub new_password: Option<String>,
    #[serde(default)]
    pub remove_protection: bool,
    #[serde(default)]
    pub commit: bool,
}

#[derive(Debug, Serialize)]
pub struct SaveEditorResult {
    pub note_id: Option<String>,
    pub protected: bool,
    pub committed: bool,
    pub saved_at: chrono::DateTime<Utc>,
}

#[derive(Serialize)]
pub struct DraftInfo {
    pub id: String,
    pub note_id: Option<String>,
    pub profile_id: String,
    pub protected: bool,
    pub updated_at: chrono::DateTime<Utc>,
}

fn content_from_note(note: &Note, payload: &ProtectedNote) -> EditorContent {
    EditorContent { title: payload.title.clone(), content: payload.content.clone(), attachment_name: payload.attachment_name.clone(), attachment_data: payload.attachment_data.clone(), wallpaper: note.wallpaper.clone(), glass: note.glass, tags: payload.tags.clone() }
}

pub fn open(s: &mut Session, mut params: OpenEditorParams) -> Result<EditorView, String> {
    let password = Zeroizing::new(params.password.take().unwrap_or_default());
    let (editor, content) = if let Some(id) = params.draft_id {
        let draft = s.db.drafts.iter().find(|d| d.id == id).ok_or("Brouillon introuvable")?;
        if s.editors.values().any(|e| e.draft_id == id) { return Err("Ce brouillon est déjà ouvert".into()); }
        let salt = draft.salt.as_ref().map(|salt| B64.decode(salt).map_err(|e| e.to_string())).transpose()?;
        let key = if draft.protected {
            Some(Zeroizing::new(crypto::derive_key(&password, salt.as_deref().ok_or("Brouillon corrompu")?)?))
        } else { None };
        let plain = Zeroizing::new(crypto::decrypt_aes_gcm(key.as_deref().unwrap_or(&s.key), &B64.decode(&draft.nonce).map_err(|e| e.to_string())?, &B64.decode(&draft.ciphertext).map_err(|e| e.to_string())?)?);
        let payload: DraftPayload = serde_json::from_slice(&plain).map_err(|_| "Brouillon corrompu".to_string())?;
        let note_id = if params.recover_as_new { None } else { draft.note_id.clone() };
        if let Some(note_id) = &note_id {
            let note = s.db.notes.iter().find(|n| &n.id == note_id && n.trashed_at.is_none())
                .ok_or("Note supprimée : récupérez le brouillon comme nouvelle note")?;
            if note.revision != draft.base_revision { return Err("Note modifiée depuis ce brouillon : récupérez-le comme nouvelle note".into()); }
        }
        let profile_id = params.profile_id.unwrap_or_else(|| draft.profile_id.clone());
        let mut history = payload.history;
        if params.recover_as_new { history.original = None; history.last_word_count = 0; }
        (EditorSession { draft_id: draft.id.clone(), note_id, profile_id, revision: draft.base_revision, key, salt, history }, payload.content)
    } else if let Some(id) = params.note_id {
        let note = s.db.notes.iter().find(|n| n.id == id && n.trashed_at.is_none()).ok_or("Note introuvable")?;
        let salt = note.locked_salt.as_ref().map(|salt| B64.decode(salt).map_err(|e| e.to_string())).transpose()?;
        let key = if note.is_locked {
            Some(Zeroizing::new(crypto::derive_key(&password, salt.as_deref().ok_or("Note corrompue")?)?))
        } else { None };
        let payload = if let Some(key) = &key { decrypt_note_with_key(note, key)? } else { note_payload(note) };
        let content = content_from_note(note, &payload);
        let history = EditorHistory {
            original: Some(NoteVersion { title: payload.title.clone(), content: payload.content.clone(), saved_at: Utc::now() }),
            last_word_count: payload.content.split_whitespace().count(),
            versions: payload.versions,
            highlights: payload.highlights,
            checkpointed: false,
        };
        (EditorSession { draft_id: Uuid::new_v4().to_string(), note_id: Some(id), profile_id: note.profile_id.clone(), revision: note.revision, key, salt, history }, content)
    } else {
        (EditorSession { draft_id: Uuid::new_v4().to_string(), note_id: None, profile_id: params.profile_id.ok_or("Profil requis")?, revision: 0, key: None, salt: None, history: EditorHistory::default() }, EditorContent::default())
    };
    if !s.db.profiles.iter().any(|p| p.id == editor.profile_id) { return Err("Profil supprimé : sélectionnez un profil pour récupérer ce brouillon".into()); }
    let token = Uuid::new_v4().to_string();
    let view = EditorView { token: token.clone(), draft_id: editor.draft_id.clone(), note_id: editor.note_id.clone(), profile_id: editor.profile_id.clone(), protected: editor.key.is_some(), content };
    s.editors.insert(token, editor);
    Ok(view)
}

pub fn save(s: &mut Session, mut params: SaveEditorParams) -> Result<SaveEditorResult, String> {
    let mut editor = s.editors.get(&params.token).ok_or("Édition verrouillée : rouvrez la note")?.clone();
    if let Some(profile_id) = params.profile_id {
        if profile_id != editor.profile_id {
            if editor.note_id.is_some() { return Err("Le profil d'une note existante ne peut pas être changé depuis la capture".into()); }
            if !s.db.profiles.iter().any(|p| p.id == profile_id) { return Err("Profil introuvable".into()); }
            editor.profile_id = profile_id;
        }
    }
    validate_attachment(&params.content.attachment_name, &params.content.attachment_data)?;
    params.content.tags = normalize_tags(params.content.tags);
    if params.remove_protection && params.new_password.is_some() { return Err("Choisissez le changement ou le retrait du mot de passe".into()); }
    if let Some(password) = params.new_password.take() {
        let password = Zeroizing::new(password);
        if password.chars().count() < 4 { return Err("Mot de passe de note trop court (min 4 caractères)".into()); }
        let salt = crypto::generate_salt().to_vec();
        editor.key = Some(Zeroizing::new(crypto::derive_key(&password, &salt)?));
        editor.salt = Some(salt);
    } else if params.remove_protection {
        editor.key = None;
        editor.salt = None;
    }
    let now = Utc::now();
    let payload = DraftPayload { content: params.content.clone(), history: editor.history.clone() };
    let plain = Zeroizing::new(serde_json::to_vec(&payload).map_err(|e| e.to_string())?);
    let (nonce, ciphertext) = crypto::encrypt_aes_gcm(editor.key.as_deref().unwrap_or(&s.key), &plain)?;
    let draft = NoteDraft { id: editor.draft_id.clone(), note_id: editor.note_id.clone(), profile_id: editor.profile_id.clone(), base_revision: editor.revision, updated_at: now, protected: editor.key.is_some(), salt: editor.salt.as_ref().map(|s| B64.encode(s)), nonce: B64.encode(nonce), ciphertext: B64.encode(ciphertext) };
    // Le brouillon est durable avant de tenter de modifier la note.
    mutate_database(s, |db| {
        db.drafts.retain(|d| d.id != draft.id);
        db.drafts.push(draft);
        Ok(())
    })?;
    s.editors.insert(params.token.clone(), editor.clone());
    if params.commit {
        if let Err(error) = commit(s, &mut editor, &params.content) {
            return Err(format!("Brouillon conservé — enregistrement de la note impossible : {error}"));
        }
    }
    let result = SaveEditorResult { note_id: editor.note_id.clone(), protected: editor.key.is_some(), committed: params.commit, saved_at: now };
    s.editors.insert(params.token, editor);
    Ok(result)
}

fn commit(s: &mut Session, editor: &mut EditorSession, content: &EditorContent) -> Result<(), String> {
    let mut history = editor.history.clone();
    if !history.checkpointed {
        if let Some(original) = &history.original {
            if original.title != content.title || original.content != content.content {
                history.versions.push(original.clone());
                history.checkpointed = true;
            }
        }
    }
    if history.versions.len() > 20 { history.versions.drain(0..history.versions.len() - 20); }
    let len = content.content.chars().count();
    history.highlights.retain(|h| h.start < h.end && h.end <= len);
    let now = Utc::now();
    let note_id = editor.note_id.clone().unwrap_or_else(|| Uuid::new_v4().to_string());
    let revision = editor.revision.saturating_add(1);
    mutate_database(s, |db| {
        if !db.profiles.iter().any(|p| p.id == editor.profile_id) { return Err("Profil supprimé".into()); }
        record_words(db, 0);
        let existing = if editor.note_id.is_some() {
            let note = db.notes.iter().find(|n| n.id == note_id && n.trashed_at.is_none()).ok_or("Note supprimée")?;
            if note.revision != editor.revision { return Err("La note a été modifiée : récupérez votre brouillon comme nouvelle note".into()); }
            Some(note.clone())
        } else { None };
        let mut note = Note {
            id: note_id.clone(), profile_id: editor.profile_id.clone(), revision,
            title: content.title.clone(), content: content.content.clone(),
            is_locked: false, locked_nonce: None, locked_payload: None, locked_salt: None, locked_format: 0,
            created_at: existing.as_ref().map(|n| n.created_at).unwrap_or(now),
            modified_history: existing.as_ref().map(|n| n.modified_history.clone()).unwrap_or_default(),
            attachment_name: content.attachment_name.clone(), attachment_data: content.attachment_data.clone(),
            wallpaper: content.wallpaper.clone(), glass: content.glass, tags: content.tags.clone(),
            highlights: history.highlights.clone(), versions: history.versions.clone(), trashed_at: None,
        };
        if existing.is_some() { note.modified_history.push(now); }
        if let Some(key) = &editor.key { protect_note(&mut note, key, editor.salt.as_deref().ok_or("Sel manquant")?)?; }
        else { record_words(db, content.content.split_whitespace().count().saturating_sub(editor.history.last_word_count)); }
        db.notes.retain(|n| n.id != note_id);
        db.notes.push(note);
        db.drafts.retain(|d| d.id != editor.draft_id);
        Ok(())
    })?;
    history.last_word_count = content.content.split_whitespace().count();
    editor.history = history;
    editor.note_id = Some(note_id);
    editor.revision = revision;
    Ok(())
}

pub fn list_drafts(s: &Session) -> Vec<DraftInfo> {
    let mut drafts: Vec<_> = s.db.drafts.iter().map(|d| DraftInfo { id: d.id.clone(), note_id: d.note_id.clone(), profile_id: d.profile_id.clone(), protected: d.protected, updated_at: d.updated_at }).collect();
    drafts.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    drafts
}

pub fn discard_draft(s: &mut Session, id: &str) -> Result<(), String> {
    if s.editors.values().any(|e| e.draft_id == id) { return Err("Fermez l'éditeur avant de supprimer son brouillon".into()); }
    mutate_database(s, |db| { db.drafts.retain(|d| d.id != id); Ok(()) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{sample_note, Sandbox};

    fn seed(s: &mut Session, note: Note) {
        mutate_database(s, |db| {
            db.profiles.push(Profile { id: "p".into(), name: "Perso".into(), created_at: Utc::now() });
            db.notes.push(note);
            Ok(())
        }).unwrap();
    }

    fn save_params(view: &EditorView, content: EditorContent, commit: bool) -> SaveEditorParams {
        SaveEditorParams { token: view.token.clone(), content, profile_id: None, new_password: None, remove_protection: false, commit }
    }

    #[test]
    fn autosave_coalesces_versions_and_rejects_closed_editor_tokens() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        seed(&mut s, sample_note());
        let view = open(&mut s, OpenEditorParams { note_id: Some("n".into()), ..OpenEditorParams::default() }).unwrap();
        let mut content = view.content.clone();
        content.content = "Première modification".into();
        save(&mut s, save_params(&view, content.clone(), true)).unwrap();
        content.content = "Deuxième modification, plus longue".into();
        save(&mut s, save_params(&view, content.clone(), true)).unwrap();
        assert_eq!(s.db.notes[0].content, content.content);
        assert_eq!(s.db.notes[0].versions.len(), 1);
        assert_eq!(s.db.notes[0].versions[0].content, "Contenu secret");
        assert_eq!(s.db.notes[0].revision, 2);
        assert!(s.db.drafts.is_empty());
        s.editors.remove(&view.token);
        assert!(save(&mut s, save_params(&view, content, true)).is_err());
    }

    #[test]
    fn protected_editor_authenticates_rotates_password_and_encrypts_versions_and_tags() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        let mut note = sample_note();
        let salt = [5; 16];
        let key = crypto::derive_key("initial", &salt).unwrap();
        protect_note(&mut note, &key, &salt).unwrap();
        seed(&mut s, note);
        assert!(open(&mut s, OpenEditorParams { note_id: Some("n".into()), password: Some("wrong".into()), ..OpenEditorParams::default() }).is_err());
        let view = open(&mut s, OpenEditorParams { note_id: Some("n".into()), password: Some("initial".into()), ..OpenEditorParams::default() }).unwrap();
        let mut content = view.content.clone();
        content.title = "Nouveau secret".into();
        content.content = "Texte modifié".into();
        content.tags = vec!["confidentiel".into()];
        let mut params = save_params(&view, content, true);
        params.new_password = Some("nouveau".into());
        save(&mut s, params).unwrap();
        let serialized = serde_json::to_string(&s.db.notes[0]).unwrap();
        for secret in ["Nouveau secret", "Texte modifié", "confidentiel", "Contenu secret", "secret.txt"] { assert!(!serialized.contains(secret)); }
        assert!(decrypt_note(&mut s, "n", "initial").is_err());
        let payload = decrypt_note(&mut s, "n", "nouveau").unwrap();
        assert_eq!(payload.content, "Texte modifié");
        assert_eq!(payload.tags, ["confidentiel"]);
        assert_eq!(payload.versions[0].content, "Contenu secret");
        let mut params = save_params(&view, view.content.clone(), true);
        params.remove_protection = true;
        save(&mut s, params).unwrap();
        assert!(!s.db.notes[0].is_locked);
        assert_eq!(s.db.notes[0].title, "Titre secret");
    }

    #[test]
    fn protected_new_draft_survives_session_loss_and_requires_its_password() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        seed(&mut s, sample_note());
        let view = open(&mut s, OpenEditorParams { profile_id: Some("p".into()), ..OpenEditorParams::default() }).unwrap();
        let mut content = view.content.clone();
        content.title = "Brouillon privé".into();
        content.content = "Non publié".into();
        let mut params = save_params(&view, content, false);
        params.new_password = Some("brouillon".into());
        save(&mut s, params).unwrap();
        assert_eq!(s.db.notes.len(), 1);
        assert_eq!(s.db.drafts.len(), 1);
        let draft = &s.db.drafts[0];
        assert!(crypto::decrypt_aes_gcm(&s.key, &B64.decode(&draft.nonce).unwrap(), &B64.decode(&draft.ciphertext).unwrap()).is_err());
        assert!(!serde_json::to_string(&s.db).unwrap().contains("Brouillon privé"));
        assert!(!serde_json::to_string(&list_drafts(&s)).unwrap().contains("Non publié"));
        s.editors.clear();
        let file: VaultFile = serde_json::from_slice(&fs::read(&s.vault_path).unwrap()).unwrap();
        s.db = serde_json::from_slice(&crypto::decrypt_aes_gcm(&s.key, &B64.decode(file.nonce).unwrap(), &B64.decode(file.ciphertext).unwrap()).unwrap()).unwrap();
        assert!(open(&mut s, OpenEditorParams { draft_id: Some(view.draft_id.clone()), ..OpenEditorParams::default() }).is_err());
        let recovered = open(&mut s, OpenEditorParams { draft_id: Some(view.draft_id), password: Some("brouillon".into()), ..OpenEditorParams::default() }).unwrap();
        assert_eq!(recovered.content.title, "Brouillon privé");
        let result = save(&mut s, save_params(&recovered, recovered.content.clone(), true)).unwrap();
        assert!(s.db.drafts.is_empty());
        assert_eq!(s.db.notes.len(), 2);
        let payload = decrypt_note(&mut s, result.note_id.as_deref().unwrap(), "brouillon").unwrap();
        assert_eq!(payload.content, "Non publié");
    }

    #[test]
    fn concurrent_changes_preserve_a_recoverable_draft_without_overwriting_the_note() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        seed(&mut s, sample_note());
        let view = open(&mut s, OpenEditorParams { note_id: Some("n".into()), ..OpenEditorParams::default() }).unwrap();
        mutate_database(&mut s, |db| { db.notes[0].content = "Autre modification".into(); db.notes[0].revision += 1; Ok(()) }).unwrap();
        let mut content = view.content.clone();
        content.content = "Travail à récupérer".into();
        assert!(save(&mut s, save_params(&view, content, true)).unwrap_err().contains("Brouillon conservé"));
        assert_eq!(s.db.notes[0].content, "Autre modification");
        assert_eq!(s.db.drafts.len(), 1);
        s.editors.clear();
        assert!(open(&mut s, OpenEditorParams { draft_id: Some(view.draft_id.clone()), ..OpenEditorParams::default() }).is_err());
        let recovered = open(&mut s, OpenEditorParams { draft_id: Some(view.draft_id), recover_as_new: true, ..OpenEditorParams::default() }).unwrap();
        save(&mut s, save_params(&recovered, recovered.content.clone(), true)).unwrap();
        assert_eq!(s.db.notes.len(), 2);
        assert_eq!(s.db.notes.iter().find(|n| n.id == "n").unwrap().content, "Autre modification");
        assert!(s.db.notes.iter().any(|n| n.content == "Travail à récupérer"));
    }

    #[test]
    fn failed_draft_write_is_retryable_and_drafts_require_explicit_discard() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        seed(&mut s, sample_note());
        let view = open(&mut s, OpenEditorParams { profile_id: Some("p".into()), ..OpenEditorParams::default() }).unwrap();
        let original = fs::read(&s.vault_path).unwrap();
        fs::remove_file(&s.vault_path).unwrap();
        assert!(save(&mut s, save_params(&view, view.content.clone(), false)).is_err());
        assert!(s.db.drafts.is_empty());
        atomic_write(&s.vault_path, &original).unwrap();
        save(&mut s, save_params(&view, view.content.clone(), false)).unwrap();
        assert!(discard_draft(&mut s, &view.draft_id).is_err());
        s.editors.remove(&view.token);
        discard_draft(&mut s, &view.draft_id).unwrap();
        assert!(s.db.drafts.is_empty());
    }

    #[test]
    fn protection_changes_in_a_draft_survive_restart_before_note_commit() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        seed(&mut s, sample_note());
        let view = open(&mut s, OpenEditorParams { note_id: Some("n".into()), ..OpenEditorParams::default() }).unwrap();
        let mut params = save_params(&view, view.content.clone(), false);
        params.new_password = Some("nouvelle-protection".into());
        save(&mut s, params).unwrap();
        assert!(!s.db.notes[0].is_locked);
        s.editors.clear();
        let recovered = open(&mut s, OpenEditorParams { draft_id: Some(view.draft_id), password: Some("nouvelle-protection".into()), ..OpenEditorParams::default() }).unwrap();
        save(&mut s, save_params(&recovered, recovered.content.clone(), true)).unwrap();
        assert!(s.db.notes[0].is_locked);
        let key = s.editors[&recovered.token].key.as_ref().unwrap();
        let payload = decrypt_note_with_key(&s.db.notes[0], key).unwrap();
        assert_eq!(payload.content, "Contenu secret");
        assert!(s.db.drafts.is_empty());
    }

    #[test]
    fn capture_profile_changes_are_validated_and_persist_with_the_draft() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        seed(&mut s, sample_note());
        mutate_database(&mut s, |db| { db.profiles.push(Profile { id: "other".into(), name: "Autre".into(), created_at: Utc::now() }); Ok(()) }).unwrap();
        let view = open(&mut s, OpenEditorParams { profile_id: Some("p".into()), ..OpenEditorParams::default() }).unwrap();
        let mut params = save_params(&view, view.content.clone(), false);
        params.profile_id = Some("missing".into());
        assert!(save(&mut s, params).is_err());
        assert!(s.db.drafts.is_empty());
        let mut params = save_params(&view, view.content.clone(), false);
        params.profile_id = Some("other".into());
        save(&mut s, params).unwrap();
        assert_eq!(s.db.drafts[0].profile_id, "other");
        save(&mut s, save_params(&view, view.content.clone(), true)).unwrap();
        assert_eq!(s.db.notes.last().unwrap().profile_id, "other");
        let existing = open(&mut s, OpenEditorParams { note_id: Some("n".into()), ..OpenEditorParams::default() }).unwrap();
        let mut params = save_params(&existing, existing.content.clone(), true);
        params.profile_id = Some("other".into());
        assert!(save(&mut s, params).is_err());
    }
}
