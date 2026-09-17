mod crypto;
mod models;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::{Utc, Duration};
use models::*;
use std::{collections::{HashMap, HashSet}, fs, path::PathBuf, sync::{Mutex, MutexGuard}};
use std::time::Instant;
use tauri::State;
use uuid::Uuid;
use zeroize::Zeroize;

struct Session {
    key: [u8; 32],
    vault_path: PathBuf,
    db: Database,
    /// true si ouvert via le mot de passe leurre (écritures routées vers le slot leurre).
    decoy: bool,
    has_decoy: bool,
}

// La clé ne doit jamais rester en mémoire après verrouillage / expiration.
impl Drop for Session {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

struct AppInner {
    session: Option<Session>,
    last_active: Option<Instant>,
    timeout_secs: u64,
}

struct AppState(Mutex<AppInner>);

const DEFAULT_TIMEOUT_SECS: u64 = 300; // 5 min

/// Récupère la session active, applique l'auto-verrouillage par inactivité
/// et touche l'horodatage d'activité.
fn active<'a>(guard: &'a mut MutexGuard<'_, AppInner>) -> Result<&'a mut Session, String> {
    if guard.session.is_none() {
        return Err("Coffre verrouillé".into());
    }
    let expired = match guard.last_active {
        Some(t) => t.elapsed().as_secs() > guard.timeout_secs,
        None => false,
    };
    if expired {
        guard.session = None; // Drop → zeroize de la clé
        guard.last_active = None;
        return Err("Session expirée par inactivité — reconnectez-vous".into());
    }
    guard.last_active = Some(Instant::now());
    guard.session.as_mut().ok_or_else(|| "Coffre verrouillé".into())
}

fn vault_dir() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("SecureNotes")
}

fn save_session(s: &Session) -> Result<(), String> {
    let json = serde_json::to_vec(&s.db).map_err(|e| e.to_string())?;
    let (nonce, ct) = crypto::encrypt_aes_gcm(&s.key, &json)?;
    // relire le fichier pour garder sel + nom (+ slot leurre le cas échéant)
    let raw = fs::read_to_string(&s.vault_path).map_err(|e| e.to_string())?;
    let mut vf: VaultFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if s.decoy {
        if vf.decoy_salt.is_none() { return Err("Coffre leurre manquant".into()); }
        vf.decoy_nonce = Some(B64.encode(nonce));
        vf.decoy_ciphertext = Some(B64.encode(ct));
    } else {
        vf.nonce = B64.encode(nonce);
        vf.ciphertext = B64.encode(ct);
    }
    fs::write(&s.vault_path, serde_json::to_string_pretty(&vf).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Normalisation des tags : minuscules, max 24 caractères, max 12, dédupliqués.
fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for t in tags {
        let n = t.trim().to_lowercase();
        if n.is_empty() || n.chars().count() > 24 { continue; }
        if !out.contains(&n) { out.push(n); }
        if out.len() >= 12 { break; }
    }
    out
}

// ---------- VAULT ----------

#[tauri::command]
fn create_vault(name: String, password: String, state: State<AppState>) -> Result<String, String> {
    if name.trim().is_empty() { return Err("Nom de base requis".into()); }
    if password.chars().count() < 8 { return Err("Mot de passe trop court (min 8 caractères)".into()); }
    let dir = vault_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    let path = dir.join(format!("{}.snotevault", safe));
    if path.exists() { return Err("Une base avec ce nom existe déjà".into()); }

    let salt = crypto::generate_salt();
    let key = crypto::derive_key(&password, &salt)?;
    let db = Database::default();
    let json = serde_json::to_vec(&db).map_err(|e| e.to_string())?;
    let (nonce, ct) = crypto::encrypt_aes_gcm(&key, &json)?;

    let vf = VaultFile {
        version: 1,
        name: name.clone(),
        salt: B64.encode(salt),
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(ct),
        decoy_salt: None,
        decoy_nonce: None,
        decoy_ciphertext: None,
    };
    fs::write(&path, serde_json::to_string_pretty(&vf).unwrap()).map_err(|e| e.to_string())?;

    let mut inner = state.0.lock().unwrap();
    let timeout = if inner.timeout_secs == 0 { DEFAULT_TIMEOUT_SECS } else { inner.timeout_secs };
    *inner = AppInner { session: Some(Session { key, vault_path: path.clone(), db, decoy: false, has_decoy: false }), last_active: Some(Instant::now()), timeout_secs: timeout };
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn list_vaults() -> Result<Vec<(String, String)>, String> {
    let dir = vault_dir();
    if !dir.exists() { return Ok(vec![]); }
    let mut out = vec![];
    for e in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        if p.extension().map(|x| x == "snotevault").unwrap_or(false) {
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            out.push((name, p.to_string_lossy().to_string()));
        }
    }
    Ok(out)
}

#[tauri::command]
fn unlock_vault(path: String, password: String, state: State<AppState>) -> Result<String, String> {
    let raw = fs::read_to_string(&path).map_err(|_| "Base introuvable".to_string())?;
    let vf: VaultFile = serde_json::from_str(&raw).map_err(|_| "Fichier corrompu".to_string())?;
    let salt = B64.decode(&vf.salt).map_err(|e| e.to_string())?;
    let nonce = B64.decode(&vf.nonce).map_err(|e| e.to_string())?;
    let ct = B64.decode(&vf.ciphertext).map_err(|e| e.to_string())?;
    let key = crypto::derive_key(&password, &salt)?;
    let has_decoy = vf.decoy_ciphertext.is_some();
    match crypto::decrypt_aes_gcm(&key, &nonce, &ct) {
        Ok(plain) => {
            let db: Database = serde_json::from_slice(&plain).map_err(|_| "Base corrompue".to_string())?;
            let name = vf.name.clone();
            let mut inner = state.0.lock().unwrap();
            let timeout = if inner.timeout_secs == 0 { DEFAULT_TIMEOUT_SECS } else { inner.timeout_secs };
            *inner = AppInner { session: Some(Session { key, vault_path: PathBuf::from(&path), db, decoy: false, has_decoy }), last_active: Some(Instant::now()), timeout_secs: timeout };
            drop(inner);
            auto_backup(&path); // sauvegarde auto (1/h max), erreurs silencieuses
            Ok(name)
        }
        Err(_) => {
            // Tentative leurre : même message d'erreur en cas d'échec (indiscernable).
            if let (Some(ds), Some(dn), Some(dc)) = (vf.decoy_salt, vf.decoy_nonce, vf.decoy_ciphertext) {
                if let (Ok(dsalt), Ok(dnonce), Ok(dct)) = (B64.decode(&ds), B64.decode(&dn), B64.decode(&dc)) {
                    if let Ok(dkey) = crypto::derive_key(&password, &dsalt) {
                        if let Ok(plain) = crypto::decrypt_aes_gcm(&dkey, &dnonce, &dct) {
                            if let Ok(db) = serde_json::from_slice::<Database>(&plain) {
                                let name = vf.name.clone();
                                let mut inner = state.0.lock().unwrap();
                                let timeout = if inner.timeout_secs == 0 { DEFAULT_TIMEOUT_SECS } else { inner.timeout_secs };
                                *inner = AppInner { session: Some(Session { key: dkey, vault_path: PathBuf::from(path), db, decoy: true, has_decoy: true }), last_active: Some(Instant::now()), timeout_secs: timeout };
                                return Ok(name);
                            }
                        }
                    }
                }
            }
            Err("Mot de passe incorrect ou fichier corrompu".into())
        }
    }
}

// ---------- BACKUPS CHIFFRÉS ----------

fn backups_dir() -> PathBuf {
    vault_dir().join("backups")
}

fn vault_stem(path: &str) -> String {
    PathBuf::from(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "coffre".into())
}

/// Copie horodatée du .snotevault (déjà chiffré), max 10 par coffre.
fn do_backup(path: &str) -> Result<String, String> {
    let dir = backups_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = vault_stem(path);
    let dest = dir.join(format!("{}-{}.snotevault.bak", stem, Utc::now().format("%Y%m%d-%H%M%S")));
    let data = fs::read(path).map_err(|e| e.to_string())?;
    fs::write(&dest, data).map_err(|e| e.to_string())?;
    // rotation : garder les 10 plus récentes
    let mut files: Vec<_> = fs::read_dir(&dir).map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().map(|n| n.to_string_lossy().starts_with(&stem)).unwrap_or(false))
        .collect();
    files.sort_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    files.reverse();
    for old in files.into_iter().skip(10) {
        let _ = fs::remove_file(old);
    }
    Ok(dest.file_name().unwrap().to_string_lossy().to_string())
}

/// Sauvegarde auto à l'ouverture, au plus 1/heure et par coffre.
fn auto_backup(path: &str) {
    let stem = vault_stem(path);
    let dir = backups_dir();
    let recent = fs::read_dir(&dir).ok().map(|rd| {
        rd.filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().map(|n| n.to_string_lossy().starts_with(&stem)).unwrap_or(false))
            .filter_map(|p| fs::metadata(&p).and_then(|m| m.modified()).ok())
            .max()
    }).flatten();
    let stale = recent.map(|t| t.elapsed().map(|e| e.as_secs() > 3600).unwrap_or(true)).unwrap_or(true);
    if stale {
        let _ = do_backup(path);
    }
}

#[tauri::command]
fn backup_now(state: State<AppState>) -> Result<String, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if s.decoy { return Err("Fonction indisponible".into()); }
    let path = s.vault_path.to_string_lossy().to_string();
    do_backup(&path)
}

#[tauri::command]
fn list_backups(state: State<AppState>) -> Result<Vec<BackupInfo>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if s.decoy { return Ok(vec![]); }
    let dir = backups_dir();
    if !dir.exists() { return Ok(vec![]); }
    let mut out = vec![];
    for e in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        if p.extension().map(|x| x == "bak").unwrap_or(false) {
            let m = e.metadata().map_err(|e| e.to_string())?;
            let modified = m.modified().ok().map(chrono::DateTime::<Utc>::from);
            out.push(BackupInfo {
                name: p.file_name().unwrap().to_string_lossy().to_string(),
                bytes: m.len(),
                modified,
            });
        }
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(out)
}

#[tauri::command]
fn restore_backup(name: String, state: State<AppState>) -> Result<(), String> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("Nom invalide".into());
    }
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if s.decoy { return Err("Fonction indisponible".into()); }
    let src = backups_dir().join(&name);
    if !src.exists() { return Err("Sauvegarde introuvable".into()); }
    // filet de sécurité : copier l'état actuel avant restauration
    let path = s.vault_path.clone();
    drop(guard);
    let _ = do_backup(&path.to_string_lossy());
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if s.vault_path != path { return Err("Coffre changé entre-temps".into()); }
    fs::copy(&src, &path).map_err(|e| e.to_string())?;
    // recharge la base depuis le disque (même sel → même clé)
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let vf: VaultFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let nonce = B64.decode(&vf.nonce).map_err(|e| e.to_string())?;
    let ct = B64.decode(&vf.ciphertext).map_err(|e| e.to_string())?;
    let plain = crypto::decrypt_aes_gcm(&s.key, &nonce, &ct)?;
    s.db = serde_json::from_slice(&plain).map_err(|_| "Sauvegarde corrompue".to_string())?;
    Ok(())
}

// ---------- COFFRE LEURRE ----------

/// Base plausible mais fausse, ouverte sous un 2e mot de passe (protection contre contrainte).
fn decoy_database() -> Database {
    let now = Utc::now();
    let p1 = Profile { id: Uuid::new_v4().to_string(), name: "Perso".into(), created_at: now - Duration::days(120) };
    let p2 = Profile { id: Uuid::new_v4().to_string(), name: "Travail".into(), created_at: now - Duration::days(90) };
    let mk = |profile_id: String, title: &str, content: &str, days: i64, wallpaper: &str| Note {
        id: Uuid::new_v4().to_string(),
        profile_id,
        title: title.into(),
        content: content.into(),
        is_locked: false,
        locked_nonce: None,
        locked_payload: None,
        locked_salt: None,
        created_at: now - Duration::days(days),
        modified_history: vec![],
        attachment_name: None,
        attachment_data: None,
        wallpaper: wallpaper.into(),
        glass: true,
        highlights: vec![],
        versions: vec![],
        trashed_at: None,
        tags: vec![],
    };
    Database {
        profiles: vec![p1.clone(), p2.clone()],
        notes: vec![
            mk(p1.id.clone(), "Liste de courses", "Lait\nPain\nŒufs\nBeurre\nPommes", 12, "aurora"),
            mk(p1.id.clone(), "Idées week-end", "Randonnée samedi si beau temps.\nAppeler Marc pour le barbecue.", 30, "sunset"),
            mk(p2.id.clone(), "Réunion lundi", "Ordre du jour :\n- point budget\n- planning congés\n- nouveau stagiaire", 5, "ocean"),
            mk(p2.id.clone(), "Codes wifi", "Bureau : invité-2026\nImprimante : 192.168.1.20", 60, "carbon"),
        ],
        daily_goal_words: 0,
    }
}

#[tauri::command]
fn set_decoy(decoy_password: String, state: State<AppState>) -> Result<(), String> {
    if decoy_password.chars().count() < 8 {
        return Err("Mot de passe leurre trop court (min 8)".into());
    }
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if s.decoy { return Err("Action impossible dans ce coffre".into()); }
    let db = decoy_database();
    let json = serde_json::to_vec(&db).map_err(|e| e.to_string())?;
    let salt = crypto::generate_salt();
    let key = crypto::derive_key(&decoy_password, &salt)?;
    let (nonce, ct) = crypto::encrypt_aes_gcm(&key, &json)?;
    let raw = fs::read_to_string(&s.vault_path).map_err(|e| e.to_string())?;
    let mut vf: VaultFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    vf.decoy_salt = Some(B64.encode(salt));
    vf.decoy_nonce = Some(B64.encode(nonce));
    vf.decoy_ciphertext = Some(B64.encode(ct));
    fs::write(&s.vault_path, serde_json::to_string_pretty(&vf).unwrap()).map_err(|e| e.to_string())?;
    s.has_decoy = true;
    Ok(())
}

#[tauri::command]
fn remove_decoy(state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if s.decoy { return Err("Action impossible dans ce coffre".into()); }
    let raw = fs::read_to_string(&s.vault_path).map_err(|e| e.to_string())?;
    let mut vf: VaultFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    vf.decoy_salt = None;
    vf.decoy_nonce = None;
    vf.decoy_ciphertext = None;
    fs::write(&s.vault_path, serde_json::to_string_pretty(&vf).unwrap()).map_err(|e| e.to_string())?;
    s.has_decoy = false;
    Ok(())
}

#[tauri::command]
fn lock_vault(state: State<AppState>) -> Result<(), String> {
    let mut inner = state.0.lock().unwrap();
    inner.session = None; // Drop → zeroize
    inner.last_active = None;
    Ok(())
}

#[tauri::command]
fn get_auto_lock_timeout(state: State<AppState>) -> Result<u64, String> {
    Ok(state.0.lock().unwrap().timeout_secs)
}

#[tauri::command]
fn set_auto_lock_timeout(secs: u64, state: State<AppState>) -> Result<u64, String> {
    let clamped = secs.clamp(60, 3600); // 1 min … 1 h
    let mut inner = state.0.lock().unwrap();
    inner.timeout_secs = clamped;
    Ok(clamped)
}

// ---------- PROFILS ----------

#[tauri::command]
fn create_profile(name: String, state: State<AppState>) -> Result<Profile, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if name.trim().is_empty() { return Err("Nom de profil requis".into()); }
    let p = Profile { id: Uuid::new_v4().to_string(), name, created_at: Utc::now() };
    s.db.profiles.push(p.clone());
    save_session(s)?;
    Ok(p)
}

#[tauri::command]
fn list_profiles(state: State<AppState>) -> Result<Vec<Profile>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    Ok(s.db.profiles.clone())
}

#[tauri::command]
fn delete_profile(id: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    s.db.profiles.retain(|p| p.id != id);
    // Les notes du profil partent à la corbeille (récupérables), pas de destruction directe.
    let now = Utc::now();
    for n in s.db.notes.iter_mut().filter(|n| n.profile_id == id) {
        if n.trashed_at.is_none() { n.trashed_at = Some(now); }
    }
    save_session(s)?;
    Ok(())
}

// ---------- NOTES ----------

#[tauri::command]
fn create_note(
    profile_id: String,
    title: String,
    content: String,
    note_password: Option<String>,
    attachment_name: Option<String>,
    attachment_data: Option<String>,
    wallpaper: String,
    glass: bool,
    tags: Vec<String>,
    state: State<AppState>,
) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if !s.db.profiles.iter().any(|p| p.id == profile_id) {
        return Err("Profil introuvable".into());
    }
    let mut note = Note {
        id: Uuid::new_v4().to_string(),
        profile_id,
        title,
        content: content.clone(),
        is_locked: false,
        locked_nonce: None,
        locked_payload: None,
        locked_salt: None,
        created_at: Utc::now(),
        modified_history: vec![],
        attachment_name,
        attachment_data,
        wallpaper,
        glass,
        highlights: vec![],
        versions: vec![],
        trashed_at: None,
        tags: normalize_tags(tags),
    };
    // Chiffrement individuel optionnel
    if let Some(pw) = note_password {
        if !pw.is_empty() {
            if pw.chars().count() < 4 { return Err("Mot de passe de note trop court (min 4)".into()); }
            let salt = crypto::generate_salt();
            let nkey = crypto::derive_key(&pw, &salt)?;
            let (nonce, ct) = crypto::encrypt_aes_gcm(&nkey, content.as_bytes())?;
            note.content = String::new();
            note.is_locked = true;
            note.tags.clear(); // les tags en clair fuiraient les sujets : supprimés si verrouillée
            note.locked_nonce = Some(B64.encode(nonce));
            note.locked_payload = Some(B64.encode(ct));
            note.locked_salt = Some(B64.encode(salt));
        }
    }
    s.db.notes.push(note.clone());
    save_session(s)?;
    Ok(note)
}

#[tauri::command]
fn list_notes(profile_id: String, state: State<AppState>) -> Result<Vec<Note>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    Ok(s.db.notes.iter().filter(|n| n.profile_id == profile_id && n.trashed_at.is_none()).cloned().collect())
}

#[tauri::command]
fn update_note(
    id: String,
    title: String,
    content: String,
    wallpaper: String,
    glass: bool,
    tags: Vec<String>,
    state: State<AppState>,
) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let n = s.db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
    if n.is_locked { return Err("Note verrouillée : déchiffrez-la d'abord".into()); }
    if n.trashed_at.is_some() { return Err("Note dans la corbeille : restaurez-la d'abord".into()); }
    // Snapshot du contenu précédent (plafond 20 versions)
    let now = Utc::now();
    n.versions.push(NoteVersion { title: n.title.clone(), content: n.content.clone(), saved_at: now });
    if n.versions.len() > 20 { let drop = n.versions.len() - 20; n.versions.drain(0..drop); }
    n.title = title;
    n.content = content.clone();
    n.wallpaper = wallpaper;
    n.glass = glass;
    n.tags = normalize_tags(tags);
    // Les surlignages hors-limites après édition sont nettoyés
    let len = content.chars().count();
    n.highlights.retain(|h| h.start < len && h.end <= len && h.start < h.end);
    n.modified_history.push(Utc::now());
    let out = n.clone();
    save_session(s)?;
    Ok(out)
}

#[tauri::command]
fn delete_note(id: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let n = s.db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
    n.trashed_at = Some(Utc::now()); // soft-delete, récupérable
    save_session(s)?;
    Ok(())
}

// ---------- TAGS ----------

#[tauri::command]
fn set_tags(id: String, tags: Vec<String>, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let n = s.db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
    if n.is_locked { return Err("Note verrouillée : tags impossibles".into()); }
    if n.trashed_at.is_some() { return Err("Note dans la corbeille : restaurez-la d'abord".into()); }
    n.tags = normalize_tags(tags);
    let out = n.clone();
    save_session(s)?;
    Ok(out)
}

#[tauri::command]
fn list_tags(state: State<AppState>) -> Result<Vec<(String, usize)>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for n in s.db.notes.iter().filter(|n| n.trashed_at.is_none() && !n.is_locked) {
        for t in &n.tags {
            *counts.entry(t.clone()).or_insert(0) += 1;
        }
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(out)
}

// ---------- GRAPHE DE LIENS [[...]] ----------

/// Extrait les références [[Nom]] d'un texte (sans crate regex).
fn extract_links(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i + 1 < chars.len() {
        if chars[i] == '[' && chars[i + 1] == '[' {
            let rest = &chars[i + 2..];
            if let Some(end) = rest.windows(2).position(|w| w == [']', ']']) {
                let name: String = rest[..end].iter().collect();
                let name = name.trim();
                if !name.is_empty() && name.chars().count() <= 80 {
                    out.push(name.to_string());
                }
                i += 2 + end + 2;
                continue;
            } else {
                break;
            }
        }
        i += 1;
    }
    out
}

/// Extrait les personnes @nom d'un texte (100% local, sans regex).
/// Règles : @ suivi de 2..24 caractères [alnum _ - .], insensible à la casse.
/// Ex : "vu @yacine avec @lina-b" -> ["yacine", "lina-b"].
fn extract_persons(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '-' || chars[j] == '.') {
                j += 1;
            }
            let raw: String = chars[i + 1..j].iter().collect();
            let name = raw.trim_matches(|c| c == '.' || c == '-' || c == '_').to_lowercase();
            let len = name.chars().count();
            if len >= 2 && len <= 24 && !out.contains(&name) {
                out.push(name);
            }
            i = if j == i + 1 { i + 1 } else { j };
            continue;
        }
        i += 1;
    }
    out
}

/// Ancien nom (couche @mentions) : désormais = personnes.
#[allow(dead_code)]
fn extract_mentions(text: &str) -> Vec<String> {
    extract_persons(text)
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.'
}

fn clean_name(raw: &str) -> Option<String> {
    let name = raw.trim_matches(|c| c == '.' || c == '-' || c == '_').to_lowercase();
    let len = name.chars().count();
    if len >= 2 && len <= 24 {
        Some(name)
    } else {
        None
    }
}

/// Extrait les lieux : 📍Nom + #lieu/Nom, #lieux/Nom, #place/Nom.
/// Ex : "vu 📍Alger, #lieu/Oran" -> ["alger", "oran"].
fn extract_places(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = vec![];
    let mut push = |n: String| {
        if !out.contains(&n) {
            out.push(n);
        }
    };
    let mut i = 0;
    while i < chars.len() {
        // 📍Nom (espace optionnel après l'emoji)
        if chars[i] == '📍' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let mut k = j;
            while k < chars.len() && is_name_char(chars[k]) {
                k += 1;
            }
            if k > j {
                let raw: String = chars[j..k].iter().collect();
                if let Some(n) = clean_name(&raw) {
                    push(n);
                }
                i = k;
                continue;
            }
            i += 1;
            continue;
        }
        // #lieu/xxx, #lieux/xxx, #place/xxx, #places/xxx
        if chars[i] == '#' {
            let mut j = i + 1;
            while j < chars.len() && (is_name_char(chars[j]) || chars[j] == '/') {
                j += 1;
            }
            if j > i + 1 {
                let raw: String = chars[i + 1..j].iter().collect();
                let low = raw.to_lowercase();
                for prefix in ["lieux/", "lieu/", "places/", "place/"] {
                    if low.starts_with(prefix) {
                        let suffix = &low[prefix.len()..];
                        if let Some(n) = clean_name(suffix) {
                            push(n);
                        }
                        break;
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Extrait les idées : 💡Nom + #idée/Nom, #idee/Nom, #idea/Nom.
/// Ex : "💡jardinage, #idée/startup" -> ["jardinage", "startup"].
fn extract_ideas(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = vec![];
    let mut push = |n: String| {
        if !out.contains(&n) {
            out.push(n);
        }
    };
    let mut i = 0;
    while i < chars.len() {
        // 💡Nom (espace optionnel après l'emoji)
        if chars[i] == '💡' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let mut k = j;
            while k < chars.len() && is_name_char(chars[k]) {
                k += 1;
            }
            if k > j {
                let raw: String = chars[j..k].iter().collect();
                if let Some(n) = clean_name(&raw) {
                    push(n);
                }
                i = k;
                continue;
            }
            i += 1;
            continue;
        }
        // #idée/xxx, #idee/xxx, #idea/xxx, #ideas/xxx
        if chars[i] == '#' {
            let mut j = i + 1;
            while j < chars.len() && (is_name_char(chars[j]) || chars[j] == '/') {
                j += 1;
            }
            if j > i + 1 {
                let raw: String = chars[i + 1..j].iter().collect();
                let low = raw.to_lowercase();
                for prefix in ["idée/", "idee/", "ideas/", "idea/"] {
                    if low.starts_with(prefix) {
                        let suffix = &low[prefix.len()..];
                        if let Some(n) = clean_name(suffix) {
                            push(n);
                        }
                        break;
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn valid_ymd(y: u32, m: u32, d: u32) -> bool {
    if y < 1900 || y > 2100 || m < 1 || m > 12 || d < 1 || d > 31 {
        return false;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let max = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if leap { 29 } else { 28 },
        _ => 0,
    };
    d <= max
}

/// Extrait les dates explicites d'un texte, normalisées en "YYYY-MM-DD".
/// Formats : YYYY-MM-DD (sep - / .) et DD-MM-YYYY. 100% local, sans regex.
fn extract_dates(text: &str) -> Vec<String> {
    let b: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = vec![];
    let is_digit = |c: char| c.is_ascii_digit();
    let is_sep = |c: char| c == '-' || c == '/' || c == '.';
    let mut i = 0;
    while i < b.len() {
        // Motif YYYY-sep-MM-sep-DD (10 car)
        if i + 10 <= b.len()
            && b[i..i + 4].iter().all(|&c| is_digit(c))
            && is_sep(b[i + 4])
            && b[i + 5..i + 7].iter().all(|&c| is_digit(c))
            && is_sep(b[i + 7])
            && b[i + 8..i + 10].iter().all(|&c| is_digit(c))
        {
            let y: u32 = b[i..i + 4].iter().collect::<String>().parse().unwrap_or(0);
            let m: u32 = b[i + 5..i + 7].iter().collect::<String>().parse().unwrap_or(0);
            let d: u32 = b[i + 8..i + 10].iter().collect::<String>().parse().unwrap_or(0);
            if valid_ymd(y, m, d) {
                let s = format!("{y:04}-{m:02}-{d:02}");
                if !out.contains(&s) {
                    out.push(s);
                }
            }
            i += 10;
            continue;
        }
        // Motif DD-sep-MM-sep-YYYY (10 car)
        if i + 10 <= b.len()
            && b[i..i + 2].iter().all(|&c| is_digit(c))
            && is_sep(b[i + 2])
            && b[i + 3..i + 5].iter().all(|&c| is_digit(c))
            && is_sep(b[i + 5])
            && b[i + 6..i + 10].iter().all(|&c| is_digit(c))
        {
            let d: u32 = b[i..i + 2].iter().collect::<String>().parse().unwrap_or(0);
            let m: u32 = b[i + 3..i + 5].iter().collect::<String>().parse().unwrap_or(0);
            let y: u32 = b[i + 6..i + 10].iter().collect::<String>().parse().unwrap_or(0);
            if valid_ymd(y, m, d) {
                let s = format!("{y:04}-{m:02}-{d:02}");
                if !out.contains(&s) {
                    out.push(s);
                }
            }
            i += 10;
            continue;
        }
        i += 1;
    }
    out
}

#[tauri::command]
fn get_graph(state: State<AppState>) -> Result<Graph, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    // Notes verrouillées exclues : contenu chiffré (pas de liens sortants) et titre masqué.
    let live: Vec<&Note> = s.db.notes.iter()
        .filter(|n| n.trashed_at.is_none() && !n.is_locked)
        .collect();
    let pname = |pid: &str| s.db.profiles.iter()
        .find(|p| p.id == pid).map(|p| p.name.clone()).unwrap_or_else(|| "profil supprimé".into());
    let mut by_title: HashMap<String, &str> = HashMap::new();
    let mut by_id: HashMap<&str, &str> = HashMap::new();
    for n in &live {
        by_title.entry(n.title.to_lowercase()).or_insert(n.id.as_str());
        by_id.insert(n.id.as_str(), n.id.as_str());
    }
    // --- Couche 1 : liens note -> note via [[...]] ---
    let mut edge_set: HashSet<(String, String, String)> = HashSet::new();
    for n in &live {
        for r in extract_links(&n.content) {
            let key = r.to_lowercase();
            let target = by_title.get(&key).or_else(|| by_id.get(r.as_str()).map(|x| x));
            if let Some(t) = target {
                if *t != n.id.as_str() {
                    edge_set.insert((n.id.clone(), (*t).to_string(), "link".to_string()));
                }
            }
        }
    }
    // --- Couche 2 : notes -> tags (champ tags déjà normalisé) ---
    let mut tag_degree: HashMap<String, usize> = HashMap::new();
    for n in &live {
        for tg in &n.tags {
            let tid = format!("tag:{tg}");
            edge_set.insert((n.id.clone(), tid.clone(), "tag".to_string()));
            *tag_degree.entry(tg.clone()).or_insert(0) += 1;
        }
    }
    // --- Couche 3a : notes -> @personnes ---
    let mut person_degree: HashMap<String, usize> = HashMap::new();
    // --- Couche 3b : notes -> 📍lieux / #lieu/xxx ---
    let mut place_degree: HashMap<String, usize> = HashMap::new();
    // --- Couche 3c : notes -> 💡idées / #idée/xxx ---
    let mut idea_degree: HashMap<String, usize> = HashMap::new();
    for n in &live {
        let blob = format!("{} {}", n.title, n.content);
        for m in extract_persons(&blob) {
            let mid = format!("person:{m}");
            edge_set.insert((n.id.clone(), mid.clone(), "person".to_string()));
            *person_degree.entry(m.clone()).or_insert(0) += 1;
        }
        for p in extract_places(&blob) {
            let pid = format!("place:{p}");
            edge_set.insert((n.id.clone(), pid.clone(), "place".to_string()));
            *place_degree.entry(p.clone()).or_insert(0) += 1;
        }
        for id in extract_ideas(&blob) {
            let iid = format!("idea:{id}");
            edge_set.insert((n.id.clone(), iid.clone(), "idea".to_string()));
            *idea_degree.entry(id.clone()).or_insert(0) += 1;
        }
    }
    // --- Couche 5 : notes -> dates (création + dates citées YYYY-MM-DD) ---
    let mut date_degree: HashMap<String, usize> = HashMap::new();
    for n in &live {
        let created = n.created_at.format("%Y-%m-%d").to_string();
        edge_set.insert((n.id.clone(), format!("date:{created}"), "date".to_string()));
        *date_degree.entry(created).or_insert(0) += 1;
        let blob = format!("{} {}", n.title, n.content);
        for dt in extract_dates(&blob) {
            edge_set.insert((n.id.clone(), format!("date:{dt}"), "date".to_string()));
            *date_degree.entry(dt).or_insert(0) += 1;
        }
    }
    // --- Couche 4 : co-occurrence tag <-> tag (même note) ---
    let mut cooc: HashMap<(String, String), usize> = HashMap::new();
    for n in &live {
        let mut tg = n.tags.clone();
        tg.sort();
        tg.dedup();
        for i in 0..tg.len() {
            for j in (i + 1)..tg.len() {
                let (a, b) = (tg[i].clone(), tg[j].clone());
                *cooc.entry((a, b)).or_insert(0) += 1;
            }
        }
    }
    let mut out_degree: HashMap<&str, usize> = HashMap::new();
    for (f, _, _) in &edge_set {
        *out_degree.entry(f.as_str()).or_insert(0) += 1;
    }
    let mut nodes: Vec<GraphNode> = live.iter().map(|n| GraphNode {
        id: n.id.clone(),
        title: n.title.clone(),
        profile: pname(&n.profile_id),
        links: *out_degree.get(n.id.as_str()).unwrap_or(&0),
        kind: "note".to_string(),
    }).collect();
    // Nœuds tags : links = nb de notes qui l'utilisent
    let mut tags_sorted: Vec<(String, usize)> = tag_degree.into_iter().collect();
    tags_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (tg, c) in tags_sorted {
        nodes.push(GraphNode {
            id: format!("tag:{tg}"),
            title: format!("#{tg}"),
            profile: String::new(),
            links: c,
            kind: "tag".to_string(),
        });
    }
    // Nœuds personnes @ : links = nb de notes qui les citent
    let mut persons_sorted: Vec<(String, usize)> = person_degree.into_iter().collect();
    persons_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (m, c) in persons_sorted {
        nodes.push(GraphNode {
            id: format!("person:{m}"),
            title: format!("@{m}"),
            profile: String::new(),
            links: c,
            kind: "person".to_string(),
        });
    }
    // Nœuds lieux 📍 : links = nb de notes qui les citent
    let mut places_sorted: Vec<(String, usize)> = place_degree.into_iter().collect();
    places_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (p, c) in places_sorted {
        nodes.push(GraphNode {
            id: format!("place:{p}"),
            title: format!("📍{p}"),
            profile: String::new(),
            links: c,
            kind: "place".to_string(),
        });
    }
    // Nœuds idées 💡 : links = nb de notes qui les citent
    let mut ideas_sorted: Vec<(String, usize)> = idea_degree.into_iter().collect();
    ideas_sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (id, c) in ideas_sorted {
        nodes.push(GraphNode {
            id: format!("idea:{id}"),
            title: format!("💡{id}"),
            profile: String::new(),
            links: c,
            kind: "idea".to_string(),
        });
    }
    // Nœuds dates : tri chronologique (YYYY-MM-DD trie bien en string)
    let mut dates_sorted: Vec<(String, usize)> = date_degree.into_iter().collect();
    dates_sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (dt, c) in dates_sorted {
        nodes.push(GraphNode {
            id: format!("date:{dt}"),
            title: format!("📅 {dt}"),
            profile: String::new(),
            links: c,
            kind: "date".to_string(),
        });
    }
    let mut edges: Vec<GraphEdge> = edge_set.into_iter().map(|(from, to, kind)| GraphEdge { from, to, kind, weight: 1 }).collect();
    // Arêtes de co-occurrence (poids = nb de notes communes)
    for ((a, b), w) in cooc {
        edges.push(GraphEdge {
            from: format!("tag:{a}"),
            to: format!("tag:{b}"),
            kind: "cooc".to_string(),
            weight: w,
        });
    }
    Ok(Graph { nodes, edges })
}

// ---------- OBJECTIF QUOTIDIEN ----------

#[tauri::command]
fn set_daily_goal(words: usize, state: State<AppState>) -> Result<usize, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let w = words.clamp(0, 10000);
    s.db.daily_goal_words = w;
    save_session(s)?;
    Ok(w)
}

// ---------- CORBEILLE & VERSIONS ----------

#[tauri::command]
fn list_trash(state: State<AppState>) -> Result<Vec<Note>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    Ok(s.db.notes.iter().filter(|n| n.trashed_at.is_some()).cloned().collect())
}

#[tauri::command]
fn restore_note(id: String, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let n = s.db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
    n.trashed_at = None;
    let out = n.clone();
    save_session(s)?;
    Ok(out)
}

#[tauri::command]
fn purge_note(id: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let pos = s.db.notes.iter().position(|n| n.id == id).ok_or("Note introuvable")?;
    if s.db.notes[pos].trashed_at.is_none() {
        return Err("Destruction définitive refusée : mettez d'abord la note à la corbeille".into());
    }
    s.db.notes.remove(pos);
    save_session(s)?;
    Ok(())
}

#[tauri::command]
fn empty_trash(state: State<AppState>) -> Result<usize, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let before = s.db.notes.len();
    s.db.notes.retain(|n| n.trashed_at.is_none());
    save_session(s)?;
    Ok(before - s.db.notes.len())
}

#[tauri::command]
fn restore_version(id: String, index: usize, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let n = s.db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
    if n.is_locked { return Err("Note verrouillée : restauration impossible".into()); }
    if n.trashed_at.is_some() { return Err("Note dans la corbeille : restaurez-la d'abord".into()); }
    if index >= n.versions.len() { return Err("Version introuvable".into()); }
    let now = Utc::now();
    n.versions.push(NoteVersion { title: n.title.clone(), content: n.content.clone(), saved_at: now });
    if n.versions.len() > 20 { let drop = n.versions.len() - 20; n.versions.drain(0..drop); }
    let v = n.versions[index].clone();
    n.title = v.title;
    n.content = v.content;
    let len = n.content.chars().count();
    n.highlights.retain(|h| h.start < len && h.end <= len && h.start < h.end);
    n.modified_history.push(now);
    let out = n.clone();
    save_session(s)?;
    Ok(out)
}

#[tauri::command]
fn unlock_note_content(id: String, password: String, state: State<AppState>) -> Result<String, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let n = s.db.notes.iter().find(|n| n.id == id).ok_or("Note introuvable")?;
    if !n.is_locked { return Ok(n.content.clone()); }
    let salt = B64.decode(n.locked_salt.as_ref().ok_or("Note corrompue")?).map_err(|e| e.to_string())?;
    let nonce = B64.decode(n.locked_nonce.as_ref().ok_or("Note corrompue")?).map_err(|e| e.to_string())?;
    let ct = B64.decode(n.locked_payload.as_ref().ok_or("Note corrompue")?).map_err(|e| e.to_string())?;
    let nkey = crypto::derive_key(&password, &salt)?;
    let plain = crypto::decrypt_aes_gcm(&nkey, &nonce, &ct)?;
    String::from_utf8(plain).map_err(|e| e.to_string())
}

#[tauri::command]
fn add_highlight(id: String, start: usize, end: usize, color: String, state: State<AppState>) -> Result<Note, String> {
    let allowed = ["yellow", "green", "pink", "blue", "orange"];
    if !allowed.contains(&color.as_str()) { return Err("Couleur invalide".into()); }
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let n = s.db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
    if n.is_locked { return Err("Note verrouillée : surlignage impossible".into()); }
    let len = n.content.chars().count();
    if start >= end || end > len { return Err("Sélection invalide".into()); }
    let h = Highlight { id: Uuid::new_v4().to_string(), start, end, color };
    n.highlights.push(h);
    // fusion simple : tri par start pour un rendu stable
    n.highlights.sort_by_key(|h| h.start);
    let out = n.clone();
    save_session(s)?;
    Ok(out)
}

#[tauri::command]
fn remove_highlight(id: String, highlight_id: String, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let n = s.db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
    let before = n.highlights.len();
    n.highlights.retain(|h| h.id != highlight_id);
    if n.highlights.len() == before { return Err("Surlignage introuvable".into()); }
    let out = n.clone();
    save_session(s)?;
    Ok(out)
}

#[tauri::command]
fn get_stats(state: State<AppState>) -> Result<Stats, String> {
    use chrono::Duration;
    use std::collections::HashMap;

    let mut guard = state.0.lock().unwrap();
    let timeout = guard.timeout_secs;
    let s = active(&mut guard)?;

    let live: Vec<&Note> = s.db.notes.iter().filter(|n| n.trashed_at.is_none()).collect();
    let trash_count = s.db.notes.len() - live.len();
    let locked_count = live.iter().filter(|n| n.is_locked).count();
    let words = |t: &str| t.split_whitespace().count();

    let words_total: usize = live.iter().filter(|n| !n.is_locked).map(|n| words(&n.content)).sum();
    let avg_words = if live.is_empty() { 0 } else { words_total / live.len().max(1) };
    let attachments_count = live.iter().filter(|n| n.attachment_name.is_some()).count();
    let attachments_bytes: u64 = live.iter()
        .filter_map(|n| n.attachment_data.as_ref())
        .map(|b| (b.len() as u64 * 3) / 4) // base64 → octets approximatifs
        .sum();
    let highlights_total: usize = live.iter().map(|n| n.highlights.len()).sum();
    let versions_total: usize = live.iter().map(|n| n.versions.len()).sum();
    let glass_count = live.iter().filter(|n| n.glass).count();

    let mut color_counts: HashMap<&str, usize> =
        ["yellow", "green", "pink", "blue", "orange"].into_iter().map(|c| (c, 0)).collect();
    let mut wall_counts: HashMap<String, usize> = HashMap::new();
    for n in &live {
        for h in &n.highlights {
            if let Some(c) = color_counts.get_mut(h.color.as_str()) { *c += 1; }
        }
        *wall_counts.entry(n.wallpaper.clone()).or_insert(0) += 1;
    }
    let mut highlights_by_color: Vec<(String, usize)> =
        color_counts.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    highlights_by_color.sort_by(|a, b| b.1.cmp(&a.1));
    let mut wallpapers: Vec<(String, usize)> = wall_counts.into_iter().collect();
    wallpapers.sort_by(|a, b| b.1.cmp(&a.1));

    let per_profile: Vec<ProfileStat> = s.db.profiles.iter().map(|p| ProfileStat {
        name: p.name.clone(),
        count: live.iter().filter(|n| n.profile_id == p.id).count(),
    }).collect();

    // Activité journalière (créations + modifications)
    let mut crea: HashMap<chrono::NaiveDate, usize> = HashMap::new();
    let mut modi: HashMap<chrono::NaiveDate, usize> = HashMap::new();
    for n in &live {
        *crea.entry(n.created_at.date_naive()).or_insert(0) += 1;
        for m in &n.modified_history {
            *modi.entry(m.date_naive()).or_insert(0) += 1;
        }
        for v in &n.versions {
            *modi.entry(v.saved_at.date_naive()).or_insert(0) += 1;
        }
    }
    let today = Utc::now().date_naive();
    let day_label = |d: chrono::NaiveDate| d.format("%d/%m").to_string();
    let last_30d: Vec<DayStat> = (0..30).rev().map(|i| {
        let d = today - Duration::days(i);
        DayStat {
            date: day_label(d),
            creations: *crea.get(&d).unwrap_or(&0),
            modifications: *modi.get(&d).unwrap_or(&0),
        }
    }).collect();
    let heat_84d: Vec<HeatDay> = (0..84).rev().map(|i| {
        let d = today - Duration::days(i);
        HeatDay {
            date: day_label(d),
            count: crea.get(&d).unwrap_or(&0) + modi.get(&d).unwrap_or(&0),
        }
    }).collect();

    // Plus longues notes (les verrouillées sont exclues : contenu chiffré)
    let pname = |pid: &str| s.db.profiles.iter()
        .find(|p| p.id == pid).map(|p| p.name.clone()).unwrap_or_else(|| "profil supprimé".into());
    let mut longest: Vec<TopNote> = live.iter()
        .filter(|n| !n.is_locked)
        .map(|n| TopNote { title: n.title.clone(), profile: pname(&n.profile_id), words: words(&n.content) })
        .collect();
    longest.sort_by(|a, b| b.words.cmp(&a.words));
    longest.truncate(5);

    let oldest = live.iter().map(|n| n.created_at).min();
    let newest = live.iter().map(|n| n.created_at).max();
    let vault_bytes = fs::metadata(&s.vault_path).map(|m| m.len()).unwrap_or(0);
    let vault_path = s.vault_path.to_string_lossy().to_string();

    // Streak : jours consécutifs d'activité (création ou modification)
    let active_days: HashSet<chrono::NaiveDate> =
        crea.keys().chain(modi.keys()).cloned().collect();
    let days_active = active_days.len();
    let mut streak_current = 0;
    let mut d = today;
    if !active_days.contains(&d) {
        d = d - Duration::days(1);
    }
    while active_days.contains(&d) {
        streak_current += 1;
        d = d - Duration::days(1);
    }
    let mut sorted: Vec<_> = active_days.into_iter().collect();
    sorted.sort();
    let mut streak_best = 0;
    let mut run = 0;
    let mut prev: Option<chrono::NaiveDate> = None;
    for day in sorted {
        if prev.map(|p| day == p + Duration::days(1)).unwrap_or(false) {
            run += 1;
        } else {
            run = 1;
        }
        streak_best = streak_best.max(run);
        prev = Some(day);
    }
    let words_today: usize = live.iter()
        .filter(|n| !n.is_locked && n.created_at.date_naive() == today)
        .map(|n| words(&n.content))
        .sum();

    // Top tags (notes verrouillées exclues : tags supprimés de toute façon)
    let mut tag_counts: HashMap<String, usize> = HashMap::new();
    for n in live.iter().filter(|n| !n.is_locked) {
        for t in &n.tags {
            *tag_counts.entry(t.clone()).or_insert(0) += 1;
        }
    }
    let mut top_tags: Vec<(String, usize)> = tag_counts.into_iter().collect();
    top_tags.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top_tags.truncate(15);

    let decoy = s.decoy;
    let has_decoy = s.has_decoy;

    Ok(Stats {
        profiles_count: s.db.profiles.len(),
        notes_total: live.len(),
        trash_count,
        locked_count,
        unlocked_count: live.len() - locked_count,
        attachments_count,
        attachments_bytes,
        highlights_total,
        highlights_by_color,
        versions_total,
        words_total,
        avg_words,
        glass_count,
        wallpapers,
        per_profile,
        last_30d,
        heat_84d,
        longest,
        vault_bytes,
        vault_path,
        oldest,
        newest,
        auto_lock_secs: timeout,
        streak_current,
        streak_best,
        days_active,
        words_today,
        daily_goal_words: s.db.daily_goal_words,
        top_tags,
        decoy,
        has_decoy,
    })
}

#[tauri::command]
fn search_notes(params: SearchParams, state: State<AppState>) -> Result<Vec<Note>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let q = params.query.unwrap_or_default().to_lowercase();
    Ok(s.db.notes.iter().filter(|n| {
        if n.trashed_at.is_some() { return false; }
        if let Some(pid) = &params.profile_id {
            if !pid.is_empty() && &n.profile_id != pid { return false; }
        }
        if let Some(tag) = params.tag.as_ref().filter(|t| !t.is_empty()) {
            if !n.tags.iter().any(|t| t == tag) { return false; }
        }
        if let Some(from) = params.date_from {
            if n.created_at < from { return false; }
        }
        if let Some(to) = params.date_to {
            if n.created_at > to { return false; }
        }
        if !q.is_empty() {
            // Notes verrouillées : titre masqué + contenu chiffré → non cherchables par mot-clé.
            if n.is_locked { return false; }
            let hay = format!("{} {}", n.title, n.content).to_lowercase();
            if !hay.contains(&q) { return false; }
        }
        true
    }).cloned().collect())
}

fn main() {
    tauri::Builder::default()
        .manage(AppState(Mutex::new(AppInner { session: None, last_active: None, timeout_secs: DEFAULT_TIMEOUT_SECS })))
        .invoke_handler(tauri::generate_handler![
            create_vault, list_vaults, unlock_vault, lock_vault,
            get_auto_lock_timeout, set_auto_lock_timeout,
            create_profile, list_profiles, delete_profile,
            create_note, list_notes, update_note, delete_note,
            list_trash, restore_note, purge_note, empty_trash, restore_version,
            set_tags, list_tags, get_graph, set_daily_goal,
            backup_now, list_backups, restore_backup, set_decoy, remove_decoy,
            unlock_note_content, search_notes, get_stats, add_highlight, remove_highlight
        ])
        .run(tauri::generate_context!())
        .expect("Erreur Tauri");
}
