mod crypto;
mod commands;
mod editor;
mod models;
mod search;
mod links;
mod transfer;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chrono::{Utc, Duration};
use models::*;
use std::{collections::{HashMap, HashSet}, fs, io::Write, path::{Path, PathBuf}, sync::{Mutex, MutexGuard}};
use std::time::Instant;
use tauri::{Emitter, Manager, State};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

struct Session {
    id: String,
    key: [u8; 32],
    vault_path: PathBuf,
    db: Database,
    /// true si ouvert via le mot de passe leurre (écritures routées vers le slot leurre).
    decoy: bool,
    has_decoy: bool,
    editors: HashMap<String, editor::EditorSession>,
    imports: HashMap<String, transfer::ImportPlan>,
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
    if expire_session(guard) {
        return Err("Session expirée par inactivité — reconnectez-vous".into());
    }
    guard.last_active = Some(Instant::now());
    guard.session.as_mut().ok_or_else(|| "Coffre verrouillé".into())
}

fn expire_session(inner: &mut AppInner) -> bool {
    let expired = inner.session.is_some() && inner.last_active
        .map(|t| t.elapsed().as_secs() >= inner.timeout_secs).unwrap_or(false);
    if expired {
        inner.session = None;
        inner.last_active = None;
    }
    expired
}

#[tauri::command]
fn touch_session(state: State<AppState>) -> Result<(), String> {
    active(&mut state.0.lock().unwrap())?;
    Ok(())
}

fn vault_dir() -> PathBuf {
    dirs::document_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("SecureNotes")
}

fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Chemin de coffre invalide")?;
    let temporary = parent.join(format!(".securenotes-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(data).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

fn save_database(s: &Session, db: &Database) -> Result<(), String> {
    let json = Zeroizing::new(serde_json::to_vec(db).map_err(|e| e.to_string())?);
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
    atomic_write(&s.vault_path, &serde_json::to_vec_pretty(&vf).map_err(|e| e.to_string())?)?;
    Ok(())
}

/// Les mutations ne deviennent visibles en mémoire qu'après une écriture réussie.
fn mutate_database<T>(s: &mut Session, mutate: impl FnOnce(&mut Database) -> Result<T, String>) -> Result<T, String> {
    let mut next = s.db.clone();
    let out = mutate(&mut next)?;
    save_database(s, &next)?;
    s.db = next;
    Ok(out)
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
    let password = Zeroizing::new(password);
    if name.trim().is_empty() { return Err("Nom de base requis".into()); }
    if password.chars().count() < 8 { return Err("Mot de passe trop court (min 8 caractères)".into()); }
    let dir = vault_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    let path = dir.join(format!("{}.snotevault", safe));
    if path.exists() { return Err("Une base avec ce nom existe déjà".into()); }

    let salt = crypto::generate_salt();
    let key = Zeroizing::new(crypto::derive_key(&password, &salt)?);
    let db = Database::default();
    let json = Zeroizing::new(serde_json::to_vec(&db).map_err(|e| e.to_string())?);
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
    atomic_write(&path, &serde_json::to_vec_pretty(&vf).map_err(|e| e.to_string())?)?;

    let mut inner = state.0.lock().unwrap();
    let timeout = if inner.timeout_secs == 0 { DEFAULT_TIMEOUT_SECS } else { inner.timeout_secs };
    *inner = AppInner { session: Some(Session { id: Uuid::new_v4().to_string(), key: *key, vault_path: path.clone(), db, decoy: false, has_decoy: false, editors: HashMap::new(), imports: HashMap::new() }), last_active: Some(Instant::now()), timeout_secs: timeout };
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
    let password = Zeroizing::new(password);
    let raw = fs::read_to_string(&path).map_err(|_| "Base introuvable".to_string())?;
    let vf: VaultFile = serde_json::from_str(&raw).map_err(|_| "Fichier corrompu".to_string())?;
    let salt = B64.decode(&vf.salt).map_err(|e| e.to_string())?;
    let nonce = B64.decode(&vf.nonce).map_err(|e| e.to_string())?;
    let ct = B64.decode(&vf.ciphertext).map_err(|e| e.to_string())?;
    let key = Zeroizing::new(crypto::derive_key(&password, &salt)?);
    let has_decoy = vf.decoy_ciphertext.is_some();
    match crypto::decrypt_aes_gcm(&key, &nonce, &ct) {
        Ok(plain) => {
            let plain = Zeroizing::new(plain);
            let db: Database = serde_json::from_slice(&plain).map_err(|_| "Base corrompue".to_string())?;
            let name = vf.name.clone();
            let mut inner = state.0.lock().unwrap();
            let timeout = if inner.timeout_secs == 0 { DEFAULT_TIMEOUT_SECS } else { inner.timeout_secs };
            *inner = AppInner { session: Some(Session { id: Uuid::new_v4().to_string(), key: *key, vault_path: PathBuf::from(&path), db, decoy: false, has_decoy, editors: HashMap::new(), imports: HashMap::new() }), last_active: Some(Instant::now()), timeout_secs: timeout };
            drop(inner);
            auto_backup(&path); // sauvegarde auto (1/h max), erreurs silencieuses
            Ok(name)
        }
        Err(_) => {
            // Tentative leurre : même message d'erreur en cas d'échec (indiscernable).
            if let (Some(ds), Some(dn), Some(dc)) = (vf.decoy_salt, vf.decoy_nonce, vf.decoy_ciphertext) {
                if let (Ok(dsalt), Ok(dnonce), Ok(dct)) = (B64.decode(&ds), B64.decode(&dn), B64.decode(&dc)) {
                    if let Ok(dkey) = crypto::derive_key(&password, &dsalt) {
                        let dkey = Zeroizing::new(dkey);
                        if let Ok(plain) = crypto::decrypt_aes_gcm(&dkey, &dnonce, &dct) {
                            let plain = Zeroizing::new(plain);
                            if let Ok(db) = serde_json::from_slice::<Database>(&plain) {
                                let name = vf.name.clone();
                                let mut inner = state.0.lock().unwrap();
                                let timeout = if inner.timeout_secs == 0 { DEFAULT_TIMEOUT_SECS } else { inner.timeout_secs };
                                *inner = AppInner { session: Some(Session { id: Uuid::new_v4().to_string(), key: *dkey, vault_path: PathBuf::from(path), db, decoy: true, has_decoy: true, editors: HashMap::new(), imports: HashMap::new() }), last_active: Some(Instant::now()), timeout_secs: timeout };
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

fn backup_prefix(path: &str) -> String {
    // Le suffixe horodaté est validé séparément pour éviter les préfixes ambigus.
    format!("{}-", vault_stem(path))
}

fn belongs_to_vault(name: &str, path: &str) -> bool {
    let prefix = backup_prefix(path);
    let Some(stamp) = name.strip_prefix(&prefix).and_then(|n| n.strip_suffix(".snotevault.bak")) else { return false; };
    // Noms historiques : YYYYMMDD-HHMMSS ; nouveaux : timestamp + UUID.
    stamp.len() >= 15 && stamp.as_bytes()[8] == b'-'
        && stamp.as_bytes()[..8].iter().all(|b| b.is_ascii_digit())
        && stamp.as_bytes()[9..15].iter().all(|b| b.is_ascii_digit())
        && (stamp.len() == 15 || stamp.as_bytes()[15] == b'-')
}

/// Copie horodatée du .snotevault (déjà chiffré), max 10 par coffre.
fn do_backup(path: &str) -> Result<String, String> {
    do_backup_in(path, &backups_dir())
}

fn do_backup_in(path: &str, dir: &Path) -> Result<String, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let stem = vault_stem(path);
    let dest = dir.join(format!("{}-{}-{}.snotevault.bak", stem, Utc::now().format("%Y%m%d-%H%M%S"), Uuid::new_v4()));
    let data = fs::read(path).map_err(|e| e.to_string())?;
    atomic_write(&dest, &data)?;
    // rotation : garder les 10 plus récentes
    let mut files: Vec<_> = fs::read_dir(dir).map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().map(|n| belongs_to_vault(&n.to_string_lossy(), path)).unwrap_or(false))
        .collect();
    files.sort_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    files.reverse();
    for old in files.into_iter().filter(|p| p != &dest).skip(9) {
        let _ = fs::remove_file(old);
    }
    Ok(dest.file_name().unwrap().to_string_lossy().to_string())
}

/// Sauvegarde auto à l'ouverture, au plus 1/heure et par coffre.
fn auto_backup(path: &str) {
    let dir = backups_dir();
    let recent = fs::read_dir(&dir).ok().and_then(|rd| {
        rd.filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_name().map(|n| belongs_to_vault(&n.to_string_lossy(), path)).unwrap_or(false))
            .filter_map(|p| fs::metadata(&p).and_then(|m| m.modified()).ok())
            .max()
    });
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
        if p.file_name().map(|n| belongs_to_vault(&n.to_string_lossy(), &s.vault_path.to_string_lossy())).unwrap_or(false) {
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
    restore_backup_file(s, &src, &backups_dir())?;
    Ok(())
}

fn restore_backup_file(s: &mut Session, src: &Path, backup_dir: &Path) -> Result<(), String> {
    let name = src.file_name().ok_or("Nom invalide")?.to_string_lossy();
    if !belongs_to_vault(&name, &s.vault_path.to_string_lossy()) {
        return Err("Cette sauvegarde appartient à un autre coffre".into());
    }
    let raw = fs::read(src).map_err(|e| e.to_string())?;
    let vf: VaultFile = serde_json::from_slice(&raw).map_err(|_| "Sauvegarde corrompue".to_string())?;
    let current: VaultFile = serde_json::from_slice(&fs::read(&s.vault_path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if vf.salt != current.salt || vf.version != current.version {
        return Err("Sauvegarde incompatible avec ce coffre".into());
    }
    let nonce = B64.decode(&vf.nonce).map_err(|e| e.to_string())?;
    let ct = B64.decode(&vf.ciphertext).map_err(|e| e.to_string())?;
    let plain = Zeroizing::new(crypto::decrypt_aes_gcm(&s.key, &nonce, &ct)?);
    let db: Database = serde_json::from_slice(&plain).map_err(|_| "Sauvegarde corrompue".to_string())?;
    // La sauvegarde source est déjà en mémoire : la rotation ne peut plus la perdre.
    do_backup_in(&s.vault_path.to_string_lossy(), backup_dir)?;
    atomic_write(&s.vault_path, &raw)?;
    s.db = db;
    s.editors.clear();
    s.imports.clear();
    s.has_decoy = vf.decoy_ciphertext.is_some();
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
        revision: 0,
        title: title.into(),
        content: content.into(),
        is_locked: false,
        locked_nonce: None,
        locked_payload: None,
        locked_salt: None,
        locked_format: 0,
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
        daily_word_counts: Some(HashMap::new()),
        drafts: vec![],
        smart_folders: vec![],
    }
}

#[tauri::command]
fn set_decoy(decoy_password: String, state: State<AppState>) -> Result<(), String> {
    let decoy_password = Zeroizing::new(decoy_password);
    if decoy_password.chars().count() < 8 {
        return Err("Mot de passe leurre trop court (min 8)".into());
    }
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    if s.decoy { return Err("Action impossible dans ce coffre".into()); }
    let db = decoy_database();
    let json = Zeroizing::new(serde_json::to_vec(&db).map_err(|e| e.to_string())?);
    let salt = crypto::generate_salt();
    let key = Zeroizing::new(crypto::derive_key(&decoy_password, &salt)?);
    let (nonce, ct) = crypto::encrypt_aes_gcm(&key, &json)?;
    let raw = fs::read_to_string(&s.vault_path).map_err(|e| e.to_string())?;
    let mut vf: VaultFile = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    vf.decoy_salt = Some(B64.encode(salt));
    vf.decoy_nonce = Some(B64.encode(nonce));
    vf.decoy_ciphertext = Some(B64.encode(ct));
    atomic_write(&s.vault_path, &serde_json::to_vec_pretty(&vf).map_err(|e| e.to_string())?)?;
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
    atomic_write(&s.vault_path, &serde_json::to_vec_pretty(&vf).map_err(|e| e.to_string())?)?;
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
    mutate_database(s, |db| {
        db.profiles.push(p.clone());
        Ok(p)
    })
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
    mutate_database(s, |db| {
        db.profiles.retain(|p| p.id != id);
        // Les notes du profil partent à la corbeille (récupérables), pas de destruction directe.
        let now = Utc::now();
        for n in db.notes.iter_mut().filter(|n| n.profile_id == id) {
            if n.trashed_at.is_none() { n.trashed_at = Some(now); }
        }
        Ok(())
    })
}

// ---------- NOTES ----------

#[tauri::command]
fn open_note_editor(params: editor::OpenEditorParams, state: State<AppState>) -> Result<editor::EditorView, String> {
    let mut guard = state.0.lock().unwrap();
    editor::open(active(&mut guard)?, params)
}

#[tauri::command]
fn save_note_editor(params: editor::SaveEditorParams, state: State<AppState>) -> Result<editor::SaveEditorResult, String> {
    let mut guard = state.0.lock().unwrap();
    editor::save(active(&mut guard)?, params)
}

#[tauri::command]
fn close_note_editor(token: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    active(&mut guard)?.editors.remove(&token);
    Ok(())
}

#[tauri::command]
fn list_note_drafts(state: State<AppState>) -> Result<Vec<editor::DraftInfo>, String> {
    let mut guard = state.0.lock().unwrap();
    Ok(editor::list_drafts(active(&mut guard)?))
}

#[tauri::command]
fn discard_note_draft(id: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    editor::discard_draft(active(&mut guard)?, &id)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Arguments nommés du contrat IPC Tauri.
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
    validate_attachment(&attachment_name, &attachment_data)?;
    let mut note = Note {
        id: Uuid::new_v4().to_string(),
        profile_id,
        revision: 0,
        title,
        content: content.clone(),
        is_locked: false,
        locked_nonce: None,
        locked_payload: None,
        locked_salt: None,
        locked_format: 0,
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
            let pw = Zeroizing::new(pw);
            let nkey = Zeroizing::new(crypto::derive_key(&pw, &salt)?);
            protect_note(&mut note, &nkey, &salt)?;
        }
    }
    mutate_database(s, |db| {
        record_words(db, if note.is_locked { 0 } else { note.content.split_whitespace().count() });
        db.notes.push(note.clone());
        Ok(note.for_client())
    })
}

#[tauri::command]
fn list_notes(profile_id: String, state: State<AppState>) -> Result<Vec<Note>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    Ok(s.db.notes.iter().filter(|n| n.profile_id == profile_id && n.trashed_at.is_none()).map(Note::for_client).collect())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Arguments nommés du contrat IPC Tauri.
fn update_note(
    id: String,
    title: String,
    content: String,
    wallpaper: String,
    glass: bool,
    tags: Vec<String>,
    attachment_name: Option<String>,
    attachment_data: Option<String>,
    remove_attachment: Option<bool>,
    state: State<AppState>,
) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| {
        record_words(db, 0);
        let n = db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
        if n.is_locked { return Err("Note verrouillée : déchiffrez-la d'abord".into()); }
        if n.trashed_at.is_some() { return Err("Note dans la corbeille : restaurez-la d'abord".into()); }
        let added_words = content.split_whitespace().count().saturating_sub(n.content.split_whitespace().count());
        apply_attachment(n, attachment_name, attachment_data, remove_attachment.unwrap_or(false))?;
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
        n.modified_history.push(now);
        n.revision = n.revision.saturating_add(1);
        let out = n.clone();
        record_words(db, added_words);
        Ok(out)
    })
}

fn validate_attachment(name: &Option<String>, data: &Option<String>) -> Result<(), String> {
    match (name, data) {
        (None, None) => Ok(()),
        (Some(name), Some(data)) if !name.trim().is_empty() => {
            B64.decode(data).map_err(|_| "Pièce jointe invalide".to_string())?;
            Ok(())
        }
        _ => Err("Nom et contenu de pièce jointe requis ensemble".into()),
    }
}

fn apply_attachment(note: &mut Note, name: Option<String>, data: Option<String>, remove: bool) -> Result<(), String> {
    validate_attachment(&name, &data)?;
    if remove && name.is_some() { return Err("Choisissez le remplacement ou la suppression de la pièce jointe".into()); }
    if remove || name.is_some() {
        note.attachment_name = name;
        note.attachment_data = data;
    }
    Ok(())
}

fn record_words(db: &mut Database, added: usize) {
    let today = Utc::now().date_naive();
    if db.daily_word_counts.is_none() {
        let baseline = db.notes.iter().filter(|n| !n.is_locked && n.trashed_at.is_none() && n.created_at.date_naive() == today)
            .map(|n| n.content.split_whitespace().count()).sum();
        db.daily_word_counts = Some(HashMap::from([(today.to_string(), baseline)]));
    }
    let count = db.daily_word_counts.as_mut().unwrap().entry(today.to_string()).or_default();
    *count = count.saturating_add(added);
}

#[tauri::command]
fn delete_note(id: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| {
        let n = db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
        n.trashed_at = Some(Utc::now()); // soft-delete, récupérable
        n.revision = n.revision.saturating_add(1);
        Ok(())
    })
}

// ---------- TAGS ----------

#[tauri::command]
fn set_tags(id: String, tags: Vec<String>, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| {
        let n = db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
        if n.is_locked { return Err("Note verrouillée : tags impossibles".into()); }
        if n.trashed_at.is_some() { return Err("Note dans la corbeille : restaurez-la d'abord".into()); }
        n.tags = normalize_tags(tags);
        n.revision = n.revision.saturating_add(1);
        let out = n.clone();
        Ok(out)
    })
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
    links::references(text).into_iter().map(|r| r.target).collect()
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
            if (2..=24).contains(&len) && !out.contains(&name) {
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
    if (2..=24).contains(&len) {
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
                    if let Some(suffix) = low.strip_prefix(prefix) {
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
                    if let Some(suffix) = low.strip_prefix(prefix) {
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
    (1900..=2100).contains(&y) && chrono::NaiveDate::from_ymd_opt(y as i32, m, d).is_some()
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
    let link_index = links::Index::new(&s.db);
    // --- Couche 1 : liens note -> note via [[...]] ---
    let mut edge_set: HashSet<(String, String, String)> = HashSet::new();
    for n in &live {
        for r in extract_links(&n.content) {
            if let links::Resolution::Resolved(target) = link_index.resolve(&r) {
                if target.id != n.id {
                    edge_set.insert((n.id.clone(), target.id.clone(), "link".to_string()));
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
    mutate_database(s, |db| {
        db.daily_goal_words = w;
        Ok(w)
    })
}

// ---------- CORBEILLE & VERSIONS ----------

#[tauri::command]
fn list_trash(state: State<AppState>) -> Result<Vec<Note>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    Ok(s.db.notes.iter().filter(|n| n.trashed_at.is_some()).map(Note::for_client).collect())
}

#[tauri::command]
fn restore_note(id: String, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| restore_trashed_note(db, &id))
}

fn restore_trashed_note(db: &mut Database, id: &str) -> Result<Note, String> {
    let index = db.notes.iter().position(|n| n.id == id).ok_or("Note introuvable")?;
    if !db.profiles.iter().any(|p| p.id == db.notes[index].profile_id) {
        let profile_id = if let Some(profile) = db.profiles.iter().find(|p| p.name == "Notes restaurées") {
            profile.id.clone()
        } else {
            let profile = Profile { id: Uuid::new_v4().to_string(), name: "Notes restaurées".into(), created_at: Utc::now() };
            let id = profile.id.clone();
            db.profiles.push(profile);
            id
        };
        db.notes[index].profile_id = profile_id;
    }
    let n = &mut db.notes[index];
    n.trashed_at = None;
    n.revision = n.revision.saturating_add(1);
    let out = n.for_client();
    Ok(out)
}

#[tauri::command]
fn purge_note(id: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| {
        let pos = db.notes.iter().position(|n| n.id == id).ok_or("Note introuvable")?;
        if db.notes[pos].trashed_at.is_none() {
            return Err("Destruction définitive refusée : mettez d'abord la note à la corbeille".into());
        }
        db.notes.remove(pos);
        Ok(())
    })
}

#[tauri::command]
fn empty_trash(state: State<AppState>) -> Result<usize, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| {
        let before = db.notes.len();
        db.notes.retain(|n| n.trashed_at.is_none());
        Ok(before - db.notes.len())
    })
}

#[tauri::command]
fn restore_version(id: String, index: usize, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| {
        let n = db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
        if n.is_locked { return Err("Note verrouillée : restauration impossible".into()); }
        if n.trashed_at.is_some() { return Err("Note dans la corbeille : restaurez-la d'abord".into()); }
        restore_note_version(n, index)?;
        Ok(n.for_client())
    })
}

fn restore_note_version(n: &mut Note, index: usize) -> Result<(), String> {
    let v = n.versions.get(index).ok_or("Version introuvable")?.clone();
    let now = Utc::now();
    n.versions.push(NoteVersion { title: n.title.clone(), content: n.content.clone(), saved_at: now });
    if n.versions.len() > 20 { let drop = n.versions.len() - 20; n.versions.drain(0..drop); }
    n.title = v.title;
    n.content = v.content;
    let len = n.content.chars().count();
    n.highlights.retain(|h| h.start < len && h.end <= len && h.start < h.end);
    n.modified_history.push(now);
    n.revision = n.revision.saturating_add(1);
    Ok(())
}

#[tauri::command]
fn unlock_note_content(id: String, password: String, state: State<AppState>) -> Result<ProtectedNote, String> {
    let password = Zeroizing::new(password);
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    decrypt_note(s, &id, &password)
}

fn protect_note(note: &mut Note, key: &[u8; 32], salt: &[u8]) -> Result<(), String> {
    let payload = note_payload(note);
    let json = Zeroizing::new(serde_json::to_vec(&payload).map_err(|e| e.to_string())?);
    let (nonce, ct) = crypto::encrypt_aes_gcm(key, &json)?;
    note.title.zeroize();
    note.content.zeroize();
    if let Some(data) = &mut note.attachment_data { data.zeroize(); }
    note.attachment_name = None;
    note.attachment_data = None;
    note.is_locked = true;
    note.tags.clear();
    note.versions.clear();
    note.highlights.clear();
    note.locked_format = 2;
    note.locked_nonce = Some(B64.encode(nonce));
    note.locked_payload = Some(B64.encode(ct));
    note.locked_salt = Some(B64.encode(salt));
    Ok(())
}

fn decrypt_note(s: &mut Session, id: &str, password: &str) -> Result<ProtectedNote, String> {
    let n = s.db.notes.iter().find(|n| n.id == id).ok_or("Note introuvable")?;
    if !n.is_locked { return Ok(note_payload(n)); }
    let salt = B64.decode(n.locked_salt.as_ref().ok_or("Note corrompue")?).map_err(|e| e.to_string())?;
    let nkey = Zeroizing::new(crypto::derive_key(password, &salt)?);
    let payload = decrypt_note_with_key(n, &nkey)?;
    if n.locked_format == 0 {
        mutate_database(s, |db| {
            let note = db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
            note.content = payload.content.clone();
            protect_note(note, &nkey, &salt)
        })?;
    }
    Ok(payload)
}

fn note_payload(n: &Note) -> ProtectedNote {
    ProtectedNote { title: n.title.clone(), content: n.content.clone(), attachment_name: n.attachment_name.clone(), attachment_data: n.attachment_data.clone(), tags: n.tags.clone(), versions: n.versions.clone(), highlights: n.highlights.clone() }
}

fn decrypt_note_with_key(n: &Note, key: &[u8; 32]) -> Result<ProtectedNote, String> {
    let nonce = B64.decode(n.locked_nonce.as_ref().ok_or("Note corrompue")?).map_err(|e| e.to_string())?;
    let ct = B64.decode(n.locked_payload.as_ref().ok_or("Note corrompue")?).map_err(|e| e.to_string())?;
    let plain = Zeroizing::new(crypto::decrypt_aes_gcm(key, &nonce, &ct)?);
    match n.locked_format {
        1 | 2 => serde_json::from_slice(&plain).map_err(|_| "Note corrompue".to_string()),
        0 => {
            let mut payload = note_payload(n);
            payload.content = String::from_utf8(plain.to_vec()).map_err(|e| e.to_string())?;
            Ok(payload)
        }
        _ => Err("Format de note non pris en charge".into()),
    }
}

#[tauri::command]
fn add_highlight(id: String, start: usize, end: usize, color: String, state: State<AppState>) -> Result<Note, String> {
    let allowed = ["yellow", "green", "pink", "blue", "orange"];
    if !allowed.contains(&color.as_str()) { return Err("Couleur invalide".into()); }
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| {
        let n = db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
        if n.is_locked { return Err("Note verrouillée : surlignage impossible".into()); }
        if n.trashed_at.is_some() { return Err("Note dans la corbeille : restaurez-la d'abord".into()); }
        let len = n.content.chars().count();
        if start >= end || end > len { return Err("Sélection invalide".into()); }
        let h = Highlight { id: Uuid::new_v4().to_string(), start, end, color };
        n.highlights.push(h);
        // fusion simple : tri par start pour un rendu stable
        n.highlights.sort_by_key(|h| h.start);
        n.revision = n.revision.saturating_add(1);
        let out = n.clone();
        Ok(out)
    })
}

#[tauri::command]
fn remove_highlight(id: String, highlight_id: String, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    mutate_database(s, |db| {
        let n = db.notes.iter_mut().find(|n| n.id == id).ok_or("Note introuvable")?;
        if n.is_locked { return Err("Note verrouillée : surlignage impossible".into()); }
        if n.trashed_at.is_some() { return Err("Note dans la corbeille : restaurez-la d'abord".into()); }
        let before = n.highlights.len();
        n.highlights.retain(|h| h.id != highlight_id);
        if n.highlights.len() == before { return Err("Surlignage introuvable".into()); }
        n.revision = n.revision.saturating_add(1);
        let out = n.clone();
        Ok(out)
    })
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
    let (crea, modi) = daily_activity(&live);
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
        d -= Duration::days(1);
    }
    while active_days.contains(&d) {
        streak_current += 1;
        d -= Duration::days(1);
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
    let words_today: usize = s.db.daily_word_counts.as_ref().map(|counts| counts.get(&today.to_string()).copied().unwrap_or(0)).unwrap_or_else(|| live.iter()
        .filter(|n| !n.is_locked && n.created_at.date_naive() == today)
        .map(|n| words(&n.content))
        .sum());

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

type DayCounts = HashMap<chrono::NaiveDate, usize>;

fn daily_activity(notes: &[&Note]) -> (DayCounts, DayCounts) {
    let mut creations = HashMap::new();
    let mut modifications = HashMap::new();
    for n in notes {
        *creations.entry(n.created_at.date_naive()).or_insert(0) += 1;
        // Les versions sont des snapshots de ces mêmes modifications, pas d'autres événements.
        for date in &n.modified_history {
            *modifications.entry(date.date_naive()).or_insert(0) += 1;
        }
    }
    (creations, modifications)
}

#[tauri::command]
fn search_command_items(query: String, state: State<AppState>) -> Result<commands::CommandMatches, String> {
    let mut guard = state.0.lock().unwrap();
    Ok(commands::search(&active(&mut guard)?.db, &query))
}

#[tauri::command]
fn get_note(id: String, state: State<AppState>) -> Result<Note, String> {
    let mut guard = state.0.lock().unwrap();
    let session = active(&mut guard)?;
    session.db.notes.iter().find(|n| n.id == id && n.trashed_at.is_none())
        .map(Note::for_client).ok_or_else(|| "Note introuvable".into())
}

#[tauri::command]
fn search_notes(params: SearchParams, state: State<AppState>) -> Result<Vec<Note>, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let filter = search::CompiledSearch::new(&params, Utc::now())?;
    Ok(s.db.notes.iter().filter(|n| filter.matches(n, &s.db.profiles)).map(Note::for_client).collect())
}

#[cfg(test)]
fn note_matches_search(n: &Note, params: &SearchParams, q: &str) -> bool {
    let params = SearchParams { query: Some(q.into()), ..params.clone() };
    search::CompiledSearch::new(&params, Utc::now()).unwrap().matches(n, &[])
}

#[tauri::command]
fn list_smart_folders(state: State<AppState>) -> Result<Vec<search::FolderSummary>, String> {
    let mut guard = state.0.lock().unwrap();
    Ok(search::list_folders(&active(&mut guard)?.db, Utc::now()))
}

#[tauri::command]
fn save_smart_folder(id: Option<String>, name: String, params: SearchParams, state: State<AppState>) -> Result<SmartFolder, String> {
    let mut guard = state.0.lock().unwrap();
    mutate_database(active(&mut guard)?, |db| search::save_folder(db, id, name, params, Utc::now()))
}

#[tauri::command]
fn delete_smart_folder(id: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    mutate_database(active(&mut guard)?, |db| { db.smart_folders.retain(|f| f.id != id); Ok(()) })
}

#[tauri::command]
fn run_smart_folder(id: String, state: State<AppState>) -> Result<search::FolderResults, String> {
    let mut guard = state.0.lock().unwrap();
    let s = active(&mut guard)?;
    let folder = s.db.smart_folders.iter().find(|f| f.id == id).ok_or("Dossier introuvable")?.clone();
    let filter = search::CompiledSearch::new(&folder.params, Utc::now())?;
    let notes = s.db.notes.iter().filter(|n| filter.matches(n, &s.db.profiles)).map(Note::for_client).collect();
    Ok(search::FolderResults { folder, notes })
}

#[tauri::command]
fn get_note_context(id: String, state: State<AppState>) -> Result<links::NoteContext, String> {
    let mut guard = state.0.lock().unwrap();
    links::context(&active(&mut guard)?.db, &id)
}

#[tauri::command]
fn resolve_note_reference(reference: String, state: State<AppState>) -> Result<links::ResolvedReference, String> {
    let mut guard = state.0.lock().unwrap();
    links::resolve(&active(&mut guard)?.db, &reference)
}

#[tauri::command]
fn create_note_from_reference(source_id: String, reference: String, state: State<AppState>) -> Result<String, String> {
    let mut guard = state.0.lock().unwrap();
    links::create_target(active(&mut guard)?, &source_id, &reference)
}

#[tauri::command]
fn list_transfer_notes(state: State<AppState>) -> Result<Vec<transfer::TransferNote>, String> {
    let mut guard = state.0.lock().unwrap();
    Ok(transfer::list_notes(&active(&mut guard)?.db))
}

#[tauri::command]
fn export_notes(ids: Vec<String>, format: String, password: Option<String>, state: State<AppState>) -> Result<transfer::Artifact, String> {
    let mut guard = state.0.lock().unwrap();
    transfer::export(&active(&mut guard)?.db, &ids, &format, password)
}

#[tauri::command]
fn prepare_note_import(files: Vec<transfer::InputFile>, password: Option<String>, state: State<AppState>) -> Result<transfer::Preview, String> {
    let mut guard = state.0.lock().unwrap();
    transfer::prepare(active(&mut guard)?, files, password)
}

#[tauri::command]
async fn choose_directory_import(app: tauri::AppHandle, title: Option<String>) -> Result<Option<transfer::Preview>, String> {
    use tauri_plugin_dialog::DialogExt;
    let session_id = {
        let state = app.state::<AppState>(); let mut guard = state.0.lock().unwrap();
        active(&mut guard)?.id.clone()
    };
    let (send, receive) = std::sync::mpsc::channel();
    let window = app.get_webview_window("main").ok_or("Fenêtre principale introuvable")?;
    app.dialog().file().set_parent(&window).set_title(title.unwrap_or_else(|| "SecureNotes — Importer un dossier".into())).pick_folder(move |selected| { let _ = send.send(selected); });
    let selected = tauri::async_runtime::spawn_blocking(move || receive.recv()).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let Some(selected) = selected else { return Ok(None); };
    let path = selected.into_path().map_err(|e| e.to_string())?;
    let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "Import".into());
    // La lecture du dossier ne retient ni le verrou ni la clé du coffre.
    let plan = tauri::async_runtime::spawn_blocking(move || transfer::directory_plan(&path)).await.map_err(|e| e.to_string())??;
    let state = app.state::<AppState>(); let mut guard = state.0.lock().unwrap();
    let session = active(&mut guard)?;
    Ok(Some(transfer::stage_directory(session, &session_id, plan, name)?))
}

#[tauri::command]
fn commit_note_import(token: String, destination_profile_id: Option<String>, preserve_profiles: bool, copies: bool, state: State<AppState>) -> Result<transfer::ImportResult, String> {
    let mut guard = state.0.lock().unwrap();
    transfer::commit(active(&mut guard)?, &token, destination_profile_id, preserve_profiles, copies)
}

#[tauri::command]
fn discard_note_import(token: String, state: State<AppState>) -> Result<(), String> {
    let mut guard = state.0.lock().unwrap();
    active(&mut guard)?.imports.remove(&token); Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState(Mutex::new(AppInner { session: None, last_active: None, timeout_secs: DEFAULT_TIMEOUT_SECS })))
        .setup(|app| {
            let handle = app.handle().clone();
            // L'expiration efface la clé même si l'interface est suspendue.
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let state = handle.state::<AppState>();
                let expired = state.0.lock().map(|mut inner| expire_session(&mut inner)).unwrap_or(false);
                if expired { let _ = handle.emit("vault-locked", ()); }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_vault, list_vaults, unlock_vault, lock_vault,
            get_auto_lock_timeout, set_auto_lock_timeout, touch_session,
            create_profile, list_profiles, delete_profile,
            create_note, list_notes, update_note, delete_note,
            open_note_editor, save_note_editor, close_note_editor, list_note_drafts, discard_note_draft,
            list_trash, restore_note, purge_note, empty_trash, restore_version,
            set_tags, list_tags, get_graph, set_daily_goal,
            backup_now, list_backups, restore_backup, set_decoy, remove_decoy,
            unlock_note_content, search_notes, search_command_items, get_note, get_stats, add_highlight, remove_highlight,
            list_smart_folders, save_smart_folder, delete_smart_folder, run_smart_folder,
            get_note_context, resolve_note_reference, create_note_from_reference,
            list_transfer_notes, export_notes, prepare_note_import, choose_directory_import, commit_note_import, discard_note_import
        ])
        .run(tauri::generate_context!())
        .expect("Erreur Tauri");
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) struct Sandbox(pub(super) PathBuf);
    impl Sandbox {
        pub(super) fn new() -> Self {
            let path = std::env::temp_dir().join(format!("securenotes-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub(super) fn session(&self) -> Session {
            let path = self.0.join("Perso.snotevault");
            let key = [7; 32];
            let db = Database::default();
            let (nonce, ct) = crypto::encrypt_aes_gcm(&key, &serde_json::to_vec(&db).unwrap()).unwrap();
            let vf = VaultFile { version: 1, name: "Perso".into(), salt: B64.encode([1; 16]), nonce: B64.encode(nonce), ciphertext: B64.encode(ct), decoy_salt: None, decoy_nonce: None, decoy_ciphertext: None };
            atomic_write(&path, &serde_json::to_vec(&vf).unwrap()).unwrap();
            Session { id: Uuid::new_v4().to_string(), key, vault_path: path, db, decoy: false, has_decoy: false, editors: HashMap::new(), imports: HashMap::new() }
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    pub(super) fn sample_note() -> Note {
        serde_json::from_value(serde_json::json!({
            "id": "n", "profile_id": "p", "title": "Titre secret", "content": "Contenu secret",
            "attachment_name": "secret.txt", "attachment_data": "c2VjcmV0",
            "is_locked": false, "locked_nonce": null, "locked_payload": null, "locked_salt": null,
            "created_at": "2026-10-25T12:00:00Z", "modified_history": [], "wallpaper": "aurora", "glass": true
        })).unwrap()
    }

    #[test]
    fn protected_note_encrypts_title_content_and_attachment() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        let mut note = sample_note();
        let salt = [3; 16];
        let key = crypto::derive_key("note-password", &salt).unwrap();
        protect_note(&mut note, &key, &salt).unwrap();
        assert!(note.title.is_empty());
        assert!(note.content.is_empty());
        assert!(note.attachment_data.is_none());
        assert!(note.attachment_name.is_none());
        s.db.notes.push(note);
        assert!(decrypt_note(&mut s, "n", "wrong-password").is_err());
        let opened = decrypt_note(&mut s, "n", "note-password").unwrap();
        assert_eq!(opened.title, "Titre secret");
        assert_eq!(opened.content, "Contenu secret");
        assert_eq!(opened.attachment_data.as_deref(), Some("c2VjcmV0"));
    }

    #[test]
    fn restoring_at_version_limit_uses_the_requested_version() {
        for index in [0, 9, 19] {
            let mut note = sample_note();
            note.versions = (0..20).map(|i| NoteVersion { title: format!("title-{i}"), content: format!("version-{i}"), saved_at: Utc::now() }).collect();
            restore_note_version(&mut note, index).unwrap();
            assert_eq!(note.content, format!("version-{index}"));
            assert_eq!(note.versions.len(), 20);
            assert_eq!(note.versions.last().unwrap().content, "Contenu secret");
        }
        let mut note = sample_note();
        assert!(restore_note_version(&mut note, 0).is_err());
        assert_eq!(note.content, "Contenu secret");
    }

    #[test]
    fn activity_refreshes_session_and_background_expiry_clears_it() {
        let sandbox = Sandbox::new();
        let state = Mutex::new(AppInner { session: Some(sandbox.session()), last_active: Some(Instant::now() - std::time::Duration::from_secs(59)), timeout_secs: 60 });
        let mut inner = state.lock().unwrap();
        active(&mut inner).unwrap();
        assert!(inner.last_active.unwrap().elapsed().as_secs() < 1);
        assert!(!expire_session(&mut inner));
        inner.last_active = Some(Instant::now() - std::time::Duration::from_secs(60));
        assert!(expire_session(&mut inner));
        assert!(inner.session.is_none());
        assert!(active(&mut inner).is_err());
    }

    #[test]
    fn restoring_an_orphaned_note_reuses_a_recovery_profile() {
        let mut db = Database::default();
        let mut first = sample_note();
        first.trashed_at = Some(Utc::now());
        let mut second = first.clone();
        second.id = "n2".into();
        db.notes = vec![first, second];
        let restored = restore_trashed_note(&mut db, "n").unwrap();
        assert!(restored.trashed_at.is_none());
        assert_eq!(db.profiles.len(), 1);
        assert_eq!(restored.profile_id, db.profiles[0].id);
        let second = restore_trashed_note(&mut db, "n2").unwrap();
        assert_eq!(second.profile_id, restored.profile_id);
        assert_eq!(db.profiles.len(), 1);
    }

    #[test]
    fn attachment_changes_preserve_replace_or_remove_without_partial_updates() {
        let mut note = sample_note();
        apply_attachment(&mut note, None, None, false).unwrap();
        assert_eq!(note.attachment_name.as_deref(), Some("secret.txt"));
        assert!(apply_attachment(&mut note, Some("invalid".into()), Some("not base64".into()), false).is_err());
        assert_eq!(note.attachment_name.as_deref(), Some("secret.txt"));
        apply_attachment(&mut note, Some("new.txt".into()), Some("YQ==".into()), false).unwrap();
        assert_eq!(note.attachment_data.as_deref(), Some("YQ=="));
        apply_attachment(&mut note, None, None, true).unwrap();
        assert!(note.attachment_data.is_none());
    }

    #[test]
    fn daily_word_tracking_includes_edits_to_older_notes_and_migrates_legacy_counts() {
        let today = Utc::now().date_naive().to_string();
        let mut db = Database::default();
        let mut note = sample_note();
        note.created_at = Utc::now() - Duration::days(2);
        db.notes.push(note);
        record_words(&mut db, 5);
        record_words(&mut db, 2);
        assert_eq!(db.daily_word_counts.as_ref().unwrap()[&today], 7);
        let mut legacy = Database { daily_word_counts: None, ..Database::default() };
        let mut note = sample_note();
        note.created_at = Utc::now();
        note.content = "un deux trois".into();
        legacy.notes.push(note);
        record_words(&mut legacy, 2);
        assert_eq!(legacy.daily_word_counts.as_ref().unwrap()[&today], 5);
        record_words(&mut legacy, 0);
        assert_eq!(legacy.daily_word_counts.as_ref().unwrap()[&today], 5);
    }

    #[test]
    fn atomic_write_failure_cleans_up_temporary_files() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("directory");
        fs::create_dir(&target).unwrap();
        assert!(atomic_write(&target, b"data").is_err());
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 1);
    }

    #[test]
    fn search_date_end_is_exclusive_and_locked_notes_are_not_text_searchable() {
        let mut note = sample_note();
        let params = SearchParams { date_from: Some("2026-10-25T00:00:00Z".parse().unwrap()), date_to: Some("2026-10-26T00:00:00Z".parse().unwrap()), ..SearchParams::default() };
        assert!(note_matches_search(&note, &params, "secret"));
        note.created_at = params.date_to.unwrap();
        assert!(!note_matches_search(&note, &params, ""));
        note.created_at = params.date_from.unwrap();
        assert!(note_matches_search(&note, &params, ""));
        note.is_locked = true;
        assert!(!note_matches_search(&note, &params, "secret"));
        assert!(note_matches_search(&note, &params, ""));
        note.trashed_at = Some(Utc::now());
        assert!(!note_matches_search(&note, &params, ""));
    }

    #[test]
    fn dashboard_does_not_count_version_snapshots_as_additional_modifications() {
        let mut note = sample_note();
        let now = Utc::now();
        note.modified_history.push(now);
        note.versions.push(NoteVersion { title: "old".into(), content: "old".into(), saved_at: now });
        let (creations, modifications) = daily_activity(&[&note]);
        assert_eq!(creations.values().sum::<usize>(), 1);
        assert_eq!(modifications[&now.date_naive()], 1);
    }

    #[test]
    fn legacy_locked_note_is_redacted_and_migrates_after_authentication() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        let mut note = sample_note();
        let salt = [4; 16];
        let key = crypto::derive_key("legacy-password", &salt).unwrap();
        let (nonce, ct) = crypto::encrypt_aes_gcm(&key, note.content.as_bytes()).unwrap();
        note.is_locked = true;
        note.content.clear();
        note.locked_nonce = Some(B64.encode(nonce));
        note.locked_payload = Some(B64.encode(ct));
        note.locked_salt = Some(B64.encode(salt));
        let public = note.for_client();
        assert!(public.title.is_empty());
        assert!(public.attachment_data.is_none());
        s.db.notes.push(note);
        let opened = decrypt_note(&mut s, "n", "legacy-password").unwrap();
        assert_eq!(opened.content, "Contenu secret");
        assert_eq!(opened.title, "Titre secret");
        assert_eq!(s.db.notes[0].locked_format, 2);
        assert!(s.db.notes[0].title.is_empty());
        assert!(s.db.notes[0].attachment_data.is_none());
        let vf: VaultFile = serde_json::from_slice(&fs::read(&s.vault_path).unwrap()).unwrap();
        let plain = crypto::decrypt_aes_gcm(&s.key, &B64.decode(vf.nonce).unwrap(), &B64.decode(vf.ciphertext).unwrap()).unwrap();
        let db: Database = serde_json::from_slice(&plain).unwrap();
        assert_eq!(db.notes[0].locked_format, 2);
    }

    #[test]
    fn backup_names_do_not_mix_vault_prefixes_or_panic_on_unicode() {
        let path = "/vaults/Perso.snotevault";
        assert!(belongs_to_vault("Perso-20261025-120000.snotevault.bak", path));
        assert!(!belongs_to_vault("PersoTravail-20261025-120000.snotevault.bak", path));
        assert!(!belongs_to_vault("Perso-💡💡💡💡💡.snotevault.bak", path));
    }

    #[test]
    fn backups_are_unique_and_rotation_only_removes_this_vaults_files() {
        let sandbox = Sandbox::new();
        let session = sandbox.session();
        let dir = sandbox.0.join("backups");
        fs::create_dir_all(&dir).unwrap();
        let foreign = dir.join("PersoTravail-20261025-120000.snotevault.bak");
        fs::write(&foreign, b"other vault").unwrap();
        let mut names = HashSet::new();
        for _ in 0..12 {
            names.insert(do_backup_in(&session.vault_path.to_string_lossy(), &dir).unwrap());
        }
        assert_eq!(names.len(), 12);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 11);
        assert_eq!(fs::read(foreign).unwrap(), b"other vault");
    }

    #[test]
    fn failed_persistence_does_not_commit_database_changes() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        fs::remove_file(&s.vault_path).unwrap();
        let result = mutate_database(&mut s, |db| { db.daily_goal_words = 42; Ok(()) });
        assert!(result.is_err());
        assert_eq!(s.db.daily_goal_words, 0);
    }

    #[test]
    fn restore_validates_before_overwrite_and_requires_safety_backup() {
        let sandbox = Sandbox::new();
        let mut s = sandbox.session();
        let dir = sandbox.0.join("backups");
        let name = do_backup_in(&s.vault_path.to_string_lossy(), &dir).unwrap();
        let src = dir.join(name);
        mutate_database(&mut s, |db| { db.daily_goal_words = 42; Ok(()) }).unwrap();
        let original = fs::read(&s.vault_path).unwrap();
        let good_backup = fs::read(&src).unwrap();
        fs::write(&src, b"invalid JSON").unwrap();
        assert!(restore_backup_file(&mut s, &src, &dir).is_err());
        assert_eq!(fs::read(&s.vault_path).unwrap(), original);
        assert_eq!(s.db.daily_goal_words, 42);
        let valid: VaultFile = serde_json::from_slice(&good_backup).unwrap();
        let mut incompatible = valid.clone();
        incompatible.salt = B64.encode([9; 16]);
        let mut bad_nonce = valid.clone();
        bad_nonce.nonce = B64.encode([0; 3]);
        let mut bad_ciphertext = valid;
        bad_ciphertext.ciphertext = B64.encode(b"corrupted");
        for invalid in [incompatible, bad_nonce, bad_ciphertext] {
            fs::write(&src, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert!(restore_backup_file(&mut s, &src, &dir).is_err());
            assert_eq!(fs::read(&s.vault_path).unwrap(), original);
            assert_eq!(s.db.daily_goal_words, 42);
        }
        fs::write(&src, &good_backup).unwrap();
        let blocked = sandbox.0.join("not-a-directory");
        fs::write(&blocked, b"blocked").unwrap();
        assert!(restore_backup_file(&mut s, &src, &blocked).is_err());
        assert_eq!(fs::read(&s.vault_path).unwrap(), original);
        restore_backup_file(&mut s, &src, &dir).unwrap();
        assert_eq!(s.db.daily_goal_words, 0);
        assert_eq!(fs::read(&s.vault_path).unwrap(), good_backup);
        assert!(fs::read_dir(dir).unwrap().any(|e| fs::read(e.unwrap().path()).unwrap() == original));
    }
}
