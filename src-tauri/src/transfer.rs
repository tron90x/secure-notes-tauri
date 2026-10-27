use crate::models::*;
use crate::{crypto, mutate_database, Session, B64};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use pulldown_cmark::{Event, Parser, Tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, io::{Cursor, Read, Write}, path::Path};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 96 * 1024 * 1024;
const MAX_NOTES: usize = 1000;
const MAX_FILES: usize = 4096;
const MAX_LINKS: usize = 10000;
const PORTABLE_FORMAT: &str = "securenotes-portable";
const ENCRYPTED_FORMAT: &str = "securenotes-transfer";

#[derive(Deserialize)]
pub struct InputFile { pub name: String, pub data: String }

#[derive(Serialize)]
pub struct TransferNote {
    pub id: String, pub title: String, pub profile_id: String, pub profile_name: String,
    pub is_locked: bool, pub legacy: bool,
}

pub fn list_notes(db: &Database) -> Vec<TransferNote> {
    db.notes.iter().filter(|n| n.trashed_at.is_none()).map(|n| note_summary(n, &db.profiles)).collect()
}

fn note_summary(n: &Note, profiles: &[Profile]) -> TransferNote {
    TransferNote {
        id: n.id.clone(), title: if n.is_locked { String::new() } else { n.title.chars().take(200).collect() }, profile_id: n.profile_id.clone(),
        profile_name: profiles.iter().find(|p| p.id == n.profile_id).map(|p| p.name.clone()).unwrap_or_default(),
        is_locked: n.is_locked, legacy: n.is_locked && n.locked_format == 0,
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Portable {
    format: String, version: u32, exported_at: DateTime<Utc>, profiles: Vec<Profile>, notes: Vec<Note>,
}

#[derive(Serialize, Deserialize)]
struct Envelope { format: String, version: u32, algorithm: String, salt: String, nonce: String, ciphertext: String }

#[derive(Serialize, Deserialize)]
struct ManifestEntry { note: Note, content_file: String, content_sha256: String, attachment_file: Option<String>, attachment_footer: Option<String> }

#[derive(Serialize, Deserialize)]
struct Manifest { format: String, version: u32, exported_at: DateTime<Utc>, profiles: Vec<Profile>, notes: Vec<ManifestEntry> }

#[derive(Serialize)]
pub struct Artifact { pub name: String, pub data: String, pub mime: String, pub notes_count: usize }

#[derive(Clone)]
pub struct ImportPlan {
    profiles: Vec<Profile>, notes: Vec<Note>, ignored_files: usize, resource_bundles: usize, edited_files: usize, kind: String,
}

impl Drop for ImportPlan {
    fn drop(&mut self) {
        for profile in &mut self.profiles { profile.name.zeroize(); }
        for note in &mut self.notes {
            note.title.zeroize(); note.content.zeroize();
            if let Some(name) = &mut note.attachment_name { name.zeroize(); }
            if let Some(data) = &mut note.attachment_data { data.zeroize(); }
            for tag in &mut note.tags { tag.zeroize(); }
            for version in &mut note.versions { version.title.zeroize(); version.content.zeroize(); }
        }
    }
}

#[derive(Serialize)]
pub struct Preview {
    pub source: Option<String>,
    pub token: String, pub notes_count: usize, pub protected_count: usize, pub attachments_count: usize,
    pub duplicates: usize, pub profiles_count: usize, pub ignored_files: usize, pub resource_bundles: usize,
    pub edited_files: usize, pub kind: String, pub notes: Vec<TransferNote>,
}

#[derive(Serialize)]
pub struct ImportResult { pub added: usize, pub skipped: usize, pub profile_id: Option<String> }

fn fail(message: impl ToString) -> String { message.to_string() }
fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

fn checked_path(name: &str) -> Result<String, String> {
    let name = name.replace('\\', "/");
    if name.is_empty() || name.len() > 1024 || name.starts_with('/') || name.contains(':') || name.contains('\0')
        || name.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err("Chemin de fichier invalide dans l'import".into());
    }
    Ok(name)
}

fn slug(name: &str) -> String {
    let value: String = name.chars().take(80).map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    let value = value.trim_matches('.').trim_matches('_');
    if value.is_empty() { "note".into() } else { value.into() }
}

fn write_zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let total: usize = files.iter().map(|(_, data)| data.len()).sum();
    if files.len() > MAX_FILES || total > MAX_BYTES { return Err("Archive trop volumineuse (64 Mio ou 4096 fichiers maximum)".into()); }
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).unix_permissions(0o600);
    for (path, data) in files { zip.start_file(checked_path(path)?, options).map_err(fail)?; zip.write_all(data).map_err(fail)?; }
    Ok(zip.finish().map_err(fail)?.into_inner())
}

fn read_zip(bytes: &[u8]) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).map_err(|_| "Archive ZIP invalide".to_string())?;
    if zip.len() > MAX_FILES { return Err("Trop de fichiers dans l'archive".into()); }
    let mut files = HashMap::new(); let mut total = 0usize;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(fail)?;
        if file.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000) { return Err("Les liens symboliques ne sont pas acceptés dans l'archive".into()); }
        let raw_name = file.name().map_err(fail)?.into_owned();
        if file.is_dir() { checked_path(raw_name.trim_end_matches('/'))?; continue; }
        let name = checked_path(&raw_name)?;
        let size = usize::try_from(file.size()).map_err(fail)?;
        if size > MAX_BYTES.saturating_sub(total) { return Err("Contenu décompressé trop volumineux (64 Mio maximum)".into()); }
        let mut data = Vec::new();
        (&mut file).take((MAX_BYTES - total + 1) as u64).read_to_end(&mut data).map_err(fail)?;
        total += data.len();
        if total > MAX_BYTES || files.insert(name, data).is_some() { return Err("Archive trop volumineuse ou chemin dupliqué".into()); }
    }
    Ok(files)
}

fn validate(portable: &Portable) -> Result<(), String> {
    if portable.format != PORTABLE_FORMAT || portable.version != 1 { return Err("Format d'échange non pris en charge".into()); }
    if portable.notes.is_empty() || portable.notes.len() > MAX_NOTES || portable.profiles.len() > MAX_NOTES { return Err("L'échange doit contenir de 1 à 1000 notes".into()); }
    let mut ids = HashSet::new();
    for profile in &portable.profiles {
        if profile.id.is_empty() || profile.id.len() > 128 || profile.name.trim().is_empty() || profile.name.chars().count() > 256 || !ids.insert(&profile.id) { return Err("Profil d'échange invalide ou dupliqué".into()); }
    }
    let profiles = ids; let mut ids = HashSet::new();
    for note in &portable.notes {
        if note.id.is_empty() || note.id.len() > 128 || !ids.insert(&note.id) || !profiles.contains(&note.profile_id) || note.trashed_at.is_some() { return Err("Identifiant, profil ou état de note invalide dans l'échange".into()); }
        if note.is_locked {
            if !matches!(note.locked_format, 1 | 2) || !note.title.is_empty() || !note.content.is_empty() || note.attachment_name.is_some() || note.attachment_data.is_some() || !note.tags.is_empty() || !note.highlights.is_empty() || !note.versions.is_empty() {
                return Err("Déverrouillez une fois cette ancienne note protégée pour migrer son chiffrement avant le transfert".into());
            }
            let salt = B64.decode(note.locked_salt.as_ref().ok_or("Note protégée corrompue")?).map_err(fail)?;
            let nonce = B64.decode(note.locked_nonce.as_ref().ok_or("Note protégée corrompue")?).map_err(fail)?;
            let payload = B64.decode(note.locked_payload.as_ref().ok_or("Note protégée corrompue")?).map_err(fail)?;
            if salt.len() != 16 || nonce.len() != 12 || payload.len() < 16 { return Err("Chiffrement individuel invalide".into()); }
        } else {
            crate::validate_attachment(&note.attachment_name, &note.attachment_data)?;
            if note.locked_nonce.is_some() || note.locked_payload.is_some() || note.locked_salt.is_some() { return Err("Chiffrement résiduel invalide sur une note libre".into()); }
            let len = note.content.chars().count();
            if note.content.match_indices("[[").take(MAX_LINKS + 1).count() > MAX_LINKS || note.versions.iter().any(|v| v.content.match_indices("[[").take(MAX_LINKS + 1).count() > MAX_LINKS) { return Err("Trop de références dans une note (10000 maximum)".into()); }
            if note.tags.len() > 12 || note.versions.len() > 20 || note.highlights.len() > 10000 || note.modified_history.len() > 100000
                || note.highlights.iter().any(|h| h.start >= h.end || h.end > len || !["yellow", "green", "pink", "blue", "orange"].contains(&h.color.as_str())) { return Err("Métadonnées de note invalides ou trop volumineuses".into()); }
        }
    }
    Ok(())
}

fn selected(db: &Database, ids: &[String]) -> Result<Portable, String> {
    if ids.is_empty() || ids.len() > MAX_NOTES { return Err("Sélectionnez de 1 à 1000 notes".into()); }
    let mut seen = HashSet::new(); let mut notes = vec![];
    for id in ids {
        if !seen.insert(id) { continue; }
        notes.push(db.notes.iter().find(|n| &n.id == id && n.trashed_at.is_none()).ok_or("Une note sélectionnée n'existe plus")?.clone());
    }
    let mut profiles: Vec<_> = db.profiles.iter().filter(|p| notes.iter().any(|n| n.profile_id == p.id)).cloned().collect();
    for note in &notes {
        if !profiles.iter().any(|p| p.id == note.profile_id) { profiles.push(Profile { id: note.profile_id.clone(), name: "Import".into(), created_at: Utc::now() }); }
    }
    let portable = Portable { format: PORTABLE_FORMAT.into(), version: 1, exported_at: Utc::now(), profiles, notes };
    validate(&portable)?;
    Ok(portable)
}

pub fn export(db: &Database, ids: &[String], format: &str, password: Option<String>) -> Result<Artifact, String> {
    let password = Zeroizing::new(password.unwrap_or_default());
    let portable = selected(db, ids)?;
    let plain = Zeroizing::new(serde_json::to_vec(&portable).map_err(fail)?);
    if plain.len() > MAX_BYTES { return Err("Archive trop volumineuse (64 Mio de données maximum)".into()); }
    let stamp = Utc::now().format("%Y%m%d-%H%M%S");
    let (name, mime, bytes) = match format {
        "encrypted" => {
            if password.chars().count() < 8 { return Err("Mot de passe d'archive trop court (8 caractères minimum)".into()); }
            let salt = crypto::generate_salt(); let key = Zeroizing::new(crypto::derive_key(&password, &salt)?);
            let (nonce, ct) = crypto::encrypt_aes_gcm(&key, &plain)?;
            let envelope = Envelope { format: ENCRYPTED_FORMAT.into(), version: 1, algorithm: "argon2id-aes256gcm-v1".into(), salt: B64.encode(salt), nonce: B64.encode(nonce), ciphertext: B64.encode(ct) };
            (format!("SecureNotes-{stamp}.snotetransfer"), "application/x-securenotes-transfer", serde_json::to_vec(&envelope).map_err(fail)?)
        }
        "markdown" | "text" => {
            if portable.notes.iter().any(|n| n.is_locked) { return Err("Les notes protégées nécessitent le format d'échange chiffré".into()); }
            let label = if format == "text" { "Texte" } else { "Markdown" };
            (format!("SecureNotes-{label}-{stamp}.zip"), "application/zip", export_markdown(&portable, format == "text")?)
        }
        _ => return Err("Format d'export inconnu".into()),
    };
    Ok(Artifact { name, mime: mime.into(), data: B64.encode(bytes), notes_count: portable.notes.len() })
}

fn markdown_label(value: &str) -> String { value.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]") }

fn append_text(output: &mut String, text: &str) -> Result<(), String> {
    if text.len() > MAX_BYTES.saturating_sub(output.len()) { return Err("Texte converti trop volumineux (64 Mio maximum)".into()); }
    output.push_str(text); Ok(())
}

fn present_wiki(text: &str, titles: &HashMap<String, Vec<String>>, paths: &HashMap<String, String>) -> Result<String, String> {
    let mut verbatim = vec![]; let mut block_start = None;
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => block_start = Some(range.start),
            Event::End(pulldown_cmark::TagEnd::CodeBlock) => { if let Some(start) = block_start.take() { verbatim.push(start..range.end); } }
            Event::Code(_) | Event::Html(_) | Event::InlineHtml(_) => verbatim.push(range),
            _ => (),
        }
        if verbatim.len() > MAX_LINKS { return Err("Trop de structures Markdown dans une note (10000 maximum)".into()); }
    }
    verbatim.sort_by_key(|range| range.start); let mut code = 0;
    let mut output = String::new(); let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let at = text.len() - rest.len() + start;
        while code < verbatim.len() && verbatim[code].end <= at { code += 1; }
        let literal = code < verbatim.len() && verbatim[code].contains(&at);
        append_text(&mut output, &rest[..start])?; rest = &rest[start..];
        let Some(end) = rest[2..].find("]]").map(|n| n + 4) else { break; };
        let raw = &rest[..end];
        let reference = if literal { None } else { crate::links::references(raw).into_iter().next() };
        let replacement = reference.and_then(|r| {
            let target = paths.get(&r.target).or_else(|| titles.get(&r.target.to_lowercase()).filter(|ids| ids.len() == 1).and_then(|ids| paths.get(&ids[0])))?;
            Some(format!("[{}]({})", markdown_label(&r.label), target.strip_prefix("notes/")?))
        });
        append_text(&mut output, replacement.as_deref().unwrap_or(raw))?; rest = &rest[end..];
    }
    append_text(&mut output, rest)?; Ok(output)
}

fn export_markdown(portable: &Portable, plain_text: bool) -> Result<Vec<u8>, String> {
    let extension = if plain_text { "txt" } else { "md" };
    let paths: HashMap<_, _> = portable.notes.iter().enumerate().map(|(i, n)| (n.id.clone(), format!("notes/{:04}-{}.{extension}", i + 1, slug(&n.title)))).collect();
    let mut titles: HashMap<String, Vec<String>> = HashMap::new();
    for note in &portable.notes { titles.entry(note.title.to_lowercase()).or_default().push(note.id.clone()); }
    let mut files = vec![]; let mut entries = vec![];
    for (i, original) in portable.notes.iter().enumerate() {
        let mut note = original.clone();
        let attachment_file = if let (Some(name), Some(data)) = (&note.attachment_name, &note.attachment_data) {
            let path = format!("attachments/{:04}-{}", i + 1, slug(name));
            files.push((path.clone(), B64.decode(data).map_err(fail)?)); Some(path)
        } else { None };
        let profile = portable.profiles.iter().find(|p| p.id == note.profile_id).ok_or("Profil manquant")?;
        let mut text = if plain_text { note.content.clone() } else {
            let header = format!("---\ntitle: {}\ntags: {}\ncreated: {}\nprofile: {}\nsecurenotes_id: {}\n---\n", serde_json::to_string(&note.title).map_err(fail)?, serde_json::to_string(&note.tags).map_err(fail)?, serde_json::to_string(&note.created_at).map_err(fail)?, serde_json::to_string(&profile.name).map_err(fail)?, serde_json::to_string(&note.id).map_err(fail)?);
            header + &present_wiki(&note.content, &titles, &paths)?
        };
        let footer = attachment_file.as_ref().map(|path| if plain_text { format!("\n\nPièce jointe : ../{path}\n") } else { format!("\n\n[Pièce jointe : {}](../{path})\n", markdown_label(note.attachment_name.as_deref().unwrap_or("fichier"))) });
        if let Some(footer) = &footer { text.push_str(footer); }
        note.attachment_data = None;
        let path = paths[&note.id].clone();
        entries.push(ManifestEntry { note, content_file: path.clone(), content_sha256: hash(text.as_bytes()), attachment_file, attachment_footer: footer });
        files.push((path, text.into_bytes()));
    }
    let manifest = Manifest { format: PORTABLE_FORMAT.into(), version: 1, exported_at: portable.exported_at, profiles: portable.profiles.clone(), notes: entries };
    files.push(("manifest.json".into(), serde_json::to_vec_pretty(&manifest).map_err(fail)?));
    write_zip(&files)
}

#[derive(Default)]
struct Frontmatter { title: Option<String>, tags: Option<Vec<String>>, created: Option<DateTime<Utc>>, profile: Option<String>, id: Option<String>, body: String }

fn scalar(value: &str) -> String { serde_json::from_str::<String>(value).unwrap_or_else(|_| value.trim().trim_matches('\'').to_string()) }

fn frontmatter(text: &str) -> Result<Frontmatter, String> {
    let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut result = Frontmatter { body: normalized.clone(), ..Default::default() };
    let Some(after) = normalized.strip_prefix("---\n") else { return Ok(result); };
    let Some(end) = after.find("\n---\n") else { return Ok(result); };
    if end > 16384 { return Ok(result); }
    let mut in_tags = false; let mut block_tags = vec![]; let mut invalid_created = false;
    for line in after[..end].lines() {
        if in_tags && line.trim_start().starts_with("- ") { block_tags.push(scalar(line.trim_start().trim_start_matches("- ").trim())); continue; }
        in_tags = false;
        let Some((key, value)) = line.split_once(':') else { continue; };
        let value = value.trim();
        match key.trim() {
            "title" if !matches!(value, "|" | ">") => result.title = Some(scalar(value)),
            "profile" => result.profile = Some(scalar(value)),
            "securenotes_id" => {
                let id = scalar(value);
                if id.is_empty() || id.len() > 128 || id.chars().any(|c| c.is_control() || matches!(c, '[' | ']' | '|')) { return Err("Identifiant Markdown invalide".into()); }
                result.id = Some(Uuid::parse_str(&id).map(|id| id.to_string()).unwrap_or(id));
            }
            "created" | "created_at" | "date" => {
                let value = scalar(value);
                result.created = value.parse().ok().or_else(|| chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d").ok()?.and_hms_opt(0, 0, 0).map(|d| d.and_utc()));
                invalid_created = result.created.is_none();
            }
            "tags" => {
                if value.is_empty() { in_tags = true; }
                else {
                    let tags = serde_json::from_str::<Vec<String>>(value).unwrap_or_else(|_| value.trim_matches(['[', ']']).split(',').map(|s| scalar(s.trim())).collect());
                    result.tags = Some(crate::normalize_tags(tags));
                }
            }
            _ => (),
        }
    }
    if !block_tags.is_empty() { result.tags = Some(crate::normalize_tags(block_tags)); }
    if result.id.is_some() && invalid_created { return Err("Date created invalide dans l'en-tête SecureNotes".into()); }
    // Les en-têtes tiers restent dans le contenu ; seul notre en-tête est retiré.
    if result.id.is_some() { result.body = after[end + 5..].to_string(); }
    Ok(result)
}

fn decoded_url(url: &str) -> Option<String> {
    let mut bytes = vec![]; let input = url.as_bytes(); let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' && i + 2 < input.len() {
            bytes.push(u8::from_str_radix(std::str::from_utf8(&input[i + 1..i + 3]).ok()?, 16).ok()?); i += 3;
        } else { bytes.push(input[i]); i += 1; }
    }
    String::from_utf8(bytes).ok()
}

fn local_path(base: &str, url: &str) -> Option<String> {
    let url = decoded_url(url.split('#').next()?)?;
    if url.is_empty() || url.contains(':') || url.starts_with('/') || url.contains('\\') { return None; }
    let mut parts: Vec<_> = base.split('/').collect(); parts.pop();
    for part in url.split('/') { match part { ".." => { parts.pop()?; }, "." | "" => (), _ => parts.push(part) } }
    let path = parts.join("/"); checked_path(&path).ok()
}

struct MarkdownLink { start: usize, end: usize, url: String, label: String, image: bool, complex: bool }

fn markdown_links(text: &str) -> Result<Vec<MarkdownLink>, String> {
    let mut stack: Vec<MarkdownLink> = vec![]; let mut links = vec![];
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => stack.push(MarkdownLink { start: range.start, end: range.end, url: dest_url.into_string(), label: String::new(), image: false, complex: false }),
            Event::Start(Tag::Image { dest_url, .. }) => {
                for outer in &mut stack { outer.complex = true; }
                stack.push(MarkdownLink { start: range.start, end: range.end, url: dest_url.into_string(), label: String::new(), image: true, complex: false });
            }
            Event::End(pulldown_cmark::TagEnd::Link | pulldown_cmark::TagEnd::Image) => {
                if let Some(mut link) = stack.pop() { link.end = range.end; links.push(link); if links.len() > MAX_LINKS { return Err("Trop de liens Markdown dans une note (10000 maximum)".into()); } }
            }
            Event::Text(value) => { for link in &mut stack { link.label.push_str(&value); } }
            Event::Start(_) | Event::Code(_) | Event::InlineHtml(_) => { for link in &mut stack { link.complex = true; } }
            _ => (),
        }
    }
    links.sort_by_key(|link| link.start); Ok(links)
}

fn restore_note_links(text: &str, base: &str, ids: &HashMap<String, String>) -> Result<String, String> {
    let mut output = String::new(); let mut pos = 0;
    for link in markdown_links(text)? {
        if link.image || link.complex || link.url.contains('#') || link.start < pos { continue; }
        let Some(id) = local_path(base, &link.url).and_then(|path| ids.get(&path)) else { continue; };
        append_text(&mut output, &text[pos..link.start])?;
        let label: String = link.label.replace(']', "］").chars().take(80).collect();
        append_text(&mut output, &format!("[[{id}|{label}]]"))?; pos = link.end;
    }
    append_text(&mut output, &text[pos..])?; Ok(output)
}

fn text_file(name: &str) -> bool { matches!(Path::new(name).extension().and_then(|s| s.to_str()).map(str::to_lowercase).as_deref(), Some("md" | "markdown" | "txt")) }

fn profile_for(profiles: &mut Vec<Profile>, name: &str) -> String {
    if let Some(profile) = profiles.iter().find(|p| p.name.to_lowercase() == name.to_lowercase()) { return profile.id.clone(); }
    let id = Uuid::new_v4().to_string(); profiles.push(Profile { id: id.clone(), name: name.into(), created_at: Utc::now() }); id
}

fn raw_markdown(files: &HashMap<String, Vec<u8>>) -> Result<ImportPlan, String> {
    let mut paths: Vec<_> = files.keys().filter(|p| text_file(p)).cloned().collect(); paths.sort();
    if paths.is_empty() || paths.len() > MAX_NOTES { return Err("Choisissez de 1 à 1000 fichiers Markdown ou texte UTF-8".into()); }
    let mut profiles = vec![]; let mut notes = vec![]; let mut ids = HashMap::new(); let mut used = HashSet::new();
    for path in &paths {
        let text = std::str::from_utf8(&files[path]).map_err(|_| format!("Le fichier {path} n'est pas en UTF-8"))?;
        let meta = if path.to_lowercase().ends_with(".txt") { Frontmatter { body: text.trim_start_matches('\u{feff}').replace("\r\n", "\n"), ..Default::default() } } else { frontmatter(text)? };
        let id = meta.id.unwrap_or_else(|| Uuid::new_v4().to_string());
        ids.insert(path.clone(), id.clone());
        let directory = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("Import");
        let profile_id = profile_for(&mut profiles, meta.profile.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(directory));
        let heading = meta.body.lines().find_map(|line| line.strip_prefix("# ").map(str::trim)).filter(|s| !s.is_empty());
        let title = meta.title.unwrap_or_else(|| heading.map(str::to_string).unwrap_or_else(|| Path::new(path).file_stem().unwrap_or_default().to_string_lossy().into_owned()));
        notes.push(Note { id, profile_id, revision: 0, title, content: meta.body, is_locked: false, locked_nonce: None, locked_payload: None, locked_salt: None, locked_format: 0,
            created_at: meta.created.unwrap_or_else(Utc::now), modified_history: vec![], attachment_name: None, attachment_data: None,
            wallpaper: "aurora".into(), glass: true, tags: meta.tags.unwrap_or_default(), highlights: vec![], versions: vec![], trashed_at: None });
    }
    let mut resource_bundles = 0;
    for (note, path) in notes.iter_mut().zip(&paths) {
        let resources: HashSet<_> = markdown_links(&note.content)?.into_iter().filter_map(|link| local_path(path, &link.url))
            .filter(|target| files.contains_key(target) && !ids.contains_key(target)).collect();
        let mut resources: Vec<_> = resources.into_iter().collect(); resources.sort();
        if resources.len() == 1 {
            note.attachment_name = Some(Path::new(&resources[0]).file_name().unwrap().to_string_lossy().into_owned());
            note.attachment_data = Some(B64.encode(&files[&resources[0]]));
        } else if !resources.is_empty() {
            let entries: Vec<_> = resources.iter().map(|p| (p.clone(), files[p].clone())).collect();
            note.attachment_name = Some("resources.zip".into()); note.attachment_data = Some(B64.encode(write_zip(&entries)?)); resource_bundles += 1;
        }
        used.extend(resources); used.insert(path.clone());
        note.content = restore_note_links(&note.content, path, &ids)?;
    }
    Ok(ImportPlan { profiles, notes, ignored_files: files.len() - used.len(), resource_bundles, edited_files: 0, kind: "markdown".into() })
}

fn import_manifest(files: &HashMap<String, Vec<u8>>) -> Result<ImportPlan, String> {
    let manifest: Manifest = serde_json::from_slice(files.get("manifest.json").ok_or("Manifeste absent")?).map_err(fail)?;
    if manifest.format != PORTABLE_FORMAT || manifest.version != 1 || manifest.notes.len() > MAX_NOTES { return Err("Manifeste d'échange non pris en charge".into()); }
    let mut profiles = manifest.profiles;
    let ids: HashMap<_, _> = manifest.notes.iter().map(|entry| (entry.content_file.clone(), entry.note.id.clone())).collect();
    if ids.len() != manifest.notes.len() { return Err("Fichier de note dupliqué dans le manifeste".into()); }
    let mut used = HashSet::from(["manifest.json".to_string()]); let mut notes = vec![]; let mut edited_files = 0;
    for entry in manifest.notes {
        let mut note = entry.note;
        if note.is_locked { return Err("Une note protégée doit utiliser l'archive chiffrée".into()); }
        if note.attachment_data.is_some() { return Err("Pièce jointe dupliquée dans le manifeste".into()); }
        let path = checked_path(&entry.content_file)?;
        let bytes = files.get(&path).ok_or_else(|| format!("Fichier de note absent : {path}"))?; used.insert(path.clone());
        let text = std::str::from_utf8(bytes).map_err(|_| "Fichier Markdown non UTF-8".to_string())?;
        if hash(bytes) != entry.content_sha256 {
            let meta = if path.to_lowercase().ends_with(".txt") { Frontmatter { body: text.replace("\r\n", "\n"), ..Default::default() } } else { frontmatter(text)? };
            if meta.id.as_ref().is_some_and(|id| id != &note.id) { return Err("Identifiant Markdown incohérent avec le manifeste".into()); }
            note.versions.push(NoteVersion { title: note.title.clone(), content: note.content.clone(), saved_at: manifest.exported_at });
            if note.versions.len() > 20 { note.versions.remove(0); }
            let body = entry.attachment_footer.as_ref().and_then(|footer| meta.body.strip_suffix(footer)).unwrap_or(&meta.body);
            note.content = if path.to_lowercase().ends_with(".txt") { body.into() } else { restore_note_links(body, &path, &ids)? };
            if let Some(title) = meta.title { note.title = title; }
            if let Some(tags) = meta.tags { note.tags = tags; }
            if let Some(created) = meta.created { note.created_at = created; }
            if let Some(profile) = meta.profile.filter(|name| !name.trim().is_empty()) { note.profile_id = profile_for(&mut profiles, &profile); }
            note.highlights.clear(); note.modified_history.push(Utc::now()); edited_files += 1;
        }
        if let Some(path) = entry.attachment_file {
            let path = checked_path(&path)?;
            note.attachment_data = Some(B64.encode(files.get(&path).ok_or("Pièce jointe absente du paquet")?)); used.insert(path);
        }
        notes.push(note);
    }
    Ok(ImportPlan { profiles, notes, ignored_files: files.len().saturating_sub(used.len()), resource_bundles: 0, edited_files, kind: "markdown-package".into() })
}

fn parse_files(input: Vec<InputFile>, password: Option<String>) -> Result<ImportPlan, String> {
    parse_files_with_root(input, password, true)
}

fn parse_files_with_root(input: Vec<InputFile>, password: Option<String>, strip_root: bool) -> Result<ImportPlan, String> {
    let password = Zeroizing::new(password.unwrap_or_default());
    if input.is_empty() || input.len() > MAX_FILES { return Err("Choisissez de 1 à 4096 fichiers".into()); }
    let mut files = HashMap::new(); let mut total = 0usize;
    for file in input {
        let name = checked_path(&file.name)?;
        if file.data.len() > MAX_FILE_BYTES * 4 / 3 + 8 { return Err("Fichier trop volumineux (96 Mio maximum)".into()); }
        let data = B64.decode(&file.data).map_err(|_| "Encodage de fichier invalide".to_string())?;
        total = total.checked_add(data.len()).ok_or("Import trop volumineux")?;
        if total > MAX_FILE_BYTES || files.insert(name, data).is_some() { return Err("Import trop volumineux ou chemin dupliqué".into()); }
    }
    if files.len() == 1 {
        let (name, bytes) = files.iter().next().unwrap();
        if name.to_lowercase().ends_with(".snotetransfer") {
            let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| "Archive chiffrée invalide".to_string())?;
            if envelope.format != ENCRYPTED_FORMAT || envelope.version != 1 || envelope.algorithm != "argon2id-aes256gcm-v1" { return Err("Format ou algorithme d'archive non pris en charge".into()); }
            let salt = B64.decode(envelope.salt).map_err(fail)?; let nonce = B64.decode(envelope.nonce).map_err(fail)?; let ciphertext = B64.decode(envelope.ciphertext).map_err(fail)?;
            if salt.len() != 16 || nonce.len() != 12 || ciphertext.len() > MAX_BYTES + 16 { return Err("Paramètres d'archive invalides".into()); }
            let key = Zeroizing::new(crypto::derive_key(&password, &salt)?);
            let plain = Zeroizing::new(crypto::decrypt_aes_gcm(&key, &nonce, &ciphertext)?);
            let portable: Portable = serde_json::from_slice(&plain).map_err(|_| "Contenu d'archive invalide".to_string())?;
            validate(&portable)?;
            return Ok(ImportPlan { profiles: portable.profiles, notes: portable.notes, ignored_files: 0, resource_bundles: 0, edited_files: 0, kind: "encrypted".into() });
        }
        if name.to_lowercase().ends_with(".zip") { files = read_zip(bytes)?; }
    } else if files.keys().any(|name| name.to_lowercase().ends_with(".snotetransfer") || name.to_lowercase().ends_with(".zip")) {
        // Un ZIP joint à un dossier de Markdown reste une ressource ; seul un fichier
        // d'échange chiffré ne peut pas être mélangé à d'autres documents.
        if files.keys().any(|name| name.to_lowercase().ends_with(".snotetransfer")) { return Err("Importez une archive chiffrée à la fois".into()); }
    }
    if files.values().map(Vec::len).sum::<usize>() > MAX_BYTES { return Err("Contenu d'import trop volumineux (64 Mio maximum)".into()); }
    let before_metadata = files.len();
    files.retain(|path, _| !path.split('/').any(|part| part == "__MACOSX" || part == ".DS_Store" || part == "Thumbs.db" || part.starts_with("._")));
    let mut ignored = before_metadata - files.len();
    // Un sélecteur de dossier ajoute le nom du dossier racine à chaque chemin.
    if strip_root && !files.contains_key("manifest.json") {
        let root = files.keys().next().and_then(|name| name.split_once('/').map(|(root, _)| root.to_string()));
        if let Some(root) = root.filter(|root| files.keys().all(|name| name.starts_with(&format!("{root}/")))) {
            files = files.into_iter().map(|(path, data)| (path[root.len() + 1..].to_string(), data)).collect();
        }
    }
    #[derive(Deserialize)]
    struct Signature { format: Option<String> }
    let mut manifests = vec![];
    for (path, data) in &files {
        if path == "manifest.json" || path.ends_with("/manifest.json") {
            let signature: Signature = serde_json::from_slice(data).map_err(|_| "Manifeste JSON invalide".to_string())?;
            if signature.format.as_deref() == Some(PORTABLE_FORMAT) { manifests.push(path.clone()); }
        }
    }
    if manifests.len() > 1 { return Err("Plusieurs paquets détectés : importez un paquet à la fois".into()); }
    let mut plan = if let Some(manifest) = manifests.first() {
        if let Some((prefix, _)) = manifest.rsplit_once('/') {
            let prefix = format!("{prefix}/"); let before = files.len();
            files = files.into_iter().filter_map(|(path, data)| path.strip_prefix(&prefix).map(|p| (p.to_string(), data))).collect();
            ignored += before - files.len();
        }
        import_manifest(&files)?
    } else { raw_markdown(&files)? };
    plan.ignored_files += ignored;
    let portable = Portable { format: PORTABLE_FORMAT.into(), version: 1, exported_at: Utc::now(), profiles: plan.profiles.clone(), notes: plan.notes.clone() };
    validate(&portable)?;
    if serde_json::to_vec(&portable).map_err(fail)?.len() > MAX_BYTES { return Err("Données importées trop volumineuses (64 Mio maximum)".into()); }
    Ok(plan)
}

pub fn prepare(s: &mut Session, files: Vec<InputFile>, password: Option<String>) -> Result<Preview, String> {
    let plan = parse_files(files, password)?;
    Ok(stage(s, plan))
}

pub fn stage(s: &mut Session, plan: ImportPlan) -> Preview {
    let token = Uuid::new_v4().to_string();
    let preview = Preview { source: None, token: token.clone(), notes_count: plan.notes.len(), protected_count: plan.notes.iter().filter(|n| n.is_locked).count(),
        attachments_count: plan.notes.iter().filter(|n| n.attachment_name.is_some()).count(),
        duplicates: plan.notes.iter().filter(|n| s.db.notes.iter().any(|old| old.id == n.id)).count(), profiles_count: plan.profiles.len(),
        ignored_files: plan.ignored_files, resource_bundles: plan.resource_bundles, edited_files: plan.edited_files,
        kind: plan.kind.clone(), notes: plan.notes.iter().take(100).map(|n| note_summary(n, &plan.profiles)).collect() };
    s.imports.clear(); s.imports.insert(token, plan);
    preview
}

pub fn directory_plan(path: &Path) -> Result<ImportPlan, String> {
    let root = path.canonicalize().map_err(fail)?;
    if !root.is_dir() { return Err("Choisissez un dossier".into()); }
    let mut directories = vec![(root.clone(), 0usize)]; let mut visited = HashSet::new();
    let mut files = vec![]; let mut total = 0usize; let mut ignored = 0;
    while let Some((directory, depth)) = directories.pop() {
        if depth > 64 { return Err("Dossier trop profondément imbriqué".into()); }
        let directory = directory.canonicalize().map_err(fail)?;
        if !directory.starts_with(&root) { return Err("Un chemin sort du dossier sélectionné".into()); }
        if !visited.insert(directory.clone()) { continue; }
        for entry in std::fs::read_dir(&directory).map_err(fail)? {
            let entry = entry.map_err(fail)?; let path = entry.path(); let metadata = std::fs::symlink_metadata(&path).map_err(fail)?;
            if metadata.file_type().is_symlink() { ignored += 1; continue; }
            if metadata.is_dir() { directories.push((path, depth + 1)); continue; }
            if !metadata.is_file() { ignored += 1; continue; }
            if files.len() >= MAX_FILES || metadata.len() > MAX_BYTES.saturating_sub(total) as u64 { return Err("Dossier trop volumineux (64 Mio et 4096 fichiers maximum)".into()); }
            let canonical = path.canonicalize().map_err(fail)?;
            if !canonical.starts_with(&root) { return Err("Un fichier sort du dossier sélectionné".into()); }
            let relative = path.strip_prefix(&root).map_err(fail)?.to_str().ok_or("Nom de fichier non UTF-8")?;
            let name = checked_path(relative)?;
            let mut data = vec![];
            std::fs::File::open(&canonical).map_err(fail)?.take((MAX_BYTES - total + 1) as u64).read_to_end(&mut data).map_err(fail)?;
            total += data.len(); if total > MAX_BYTES { return Err("Dossier trop volumineux".into()); }
            files.push(InputFile { name, data: B64.encode(data) });
        }
    }
    let mut plan = parse_files_with_root(files, None, false)?; plan.ignored_files += ignored; Ok(plan)
}

pub fn stage_directory(s: &mut Session, session_id: &str, plan: ImportPlan, name: String) -> Result<Preview, String> {
    if s.id != session_id { return Err("Session expirée ou coffre changé : choisissez à nouveau le dossier".into()); }
    let mut preview = stage(s, plan); preview.source = Some(name); Ok(preview)
}

type LinkEdits = Vec<(usize, usize, usize)>;

fn remap_content(text: &str, mapping: &HashMap<String, String>, titles: &HashMap<String, Vec<String>>) -> Result<(String, LinkEdits), String> {
    let mut out = String::new(); let mut pos = 0; let mut edits = vec![]; let mut source_chars = 0;
    while let Some(start) = text[pos..].find("[[").map(|i| pos + i) {
        let Some(end) = text[start + 2..].find("]]").map(|i| start + i + 4) else { break; };
        let original = &text[start..end];
        let replacement = crate::links::references(original).into_iter().next().and_then(|r| {
            let id = mapping.get(&r.target).or_else(|| titles.get(&r.target.to_lowercase()).filter(|ids| ids.len() == 1).and_then(|ids| mapping.get(&ids[0])))?;
            Some(format!("[[{id}|{}]]", r.label.replace(']', "］")))
        });
        append_text(&mut out, &text[pos..start])?; source_chars += text[pos..start].chars().count();
        let old_len = original.chars().count();
        if let Some(replacement) = replacement {
            if out.len() + replacement.len() > MAX_BYTES { return Err("Copie trop volumineuse après conversion des liens".into()); }
            edits.push((source_chars, source_chars + old_len, replacement.chars().count()));
            out.push_str(&replacement);
        } else { append_text(&mut out, original)?; }
        pos = end; source_chars += old_len;
    }
    if out.len() + text[pos..].len() > MAX_BYTES { return Err("Copie trop volumineuse après conversion des liens".into()); }
    out.push_str(&text[pos..]); Ok((out, edits))
}

pub fn commit(s: &mut Session, token: &str, destination: Option<String>, preserve_profiles: bool, copies: bool) -> Result<ImportResult, String> {
    let plan = s.imports.get(token).ok_or("Préparation d'import introuvable : préparez à nouveau les fichiers")?.clone();
    if copies && plan.notes.iter().any(|n| n.is_locked) { return Err("Les notes protégées conservent leurs identifiants ; utilisez le mode sans écrasement".into()); }
    let mut mapping = HashMap::new(); let mut titles: HashMap<String, Vec<String>> = HashMap::new();
    if copies {
        for note in &plan.notes { mapping.insert(note.id.clone(), Uuid::new_v4().to_string()); titles.entry(note.title.to_lowercase()).or_default().push(note.id.clone()); }
    }
    let result = mutate_database(s, |db| {
        if !preserve_profiles && !db.profiles.iter().any(|p| Some(&p.id) == destination.as_ref()) { return Err("Choisissez un profil de destination".into()); }
        let mut added = 0; let mut skipped = 0; let mut first_profile = None; let mut profile_ids: HashMap<String, String> = HashMap::new();
        let mut copied_bytes = 0usize;
        for original in &plan.notes {
            if !copies && db.notes.iter().any(|n| n.id == original.id) { skipped += 1; continue; }
            let mut note = original.clone();
            if copies {
                note.id = mapping[&original.id].clone();
                let (content, edits) = remap_content(&note.content, &mapping, &titles)?;
                let offset = |position: usize, end: bool| {
                    let mut delta = 0isize;
                    for &(start, old_end, new_len) in &edits {
                        if position <= start { break; }
                        if position < old_end { return (start as isize + delta) as usize + if end { new_len } else { 0 }; }
                        delta += new_len as isize - (old_end - start) as isize;
                    }
                    (position as isize + delta) as usize
                };
                for highlight in &mut note.highlights { highlight.start = offset(highlight.start, false); highlight.end = offset(highlight.end, true); }
                note.content = content;
                copied_bytes += note.content.len() + note.attachment_data.as_ref().map_or(0, String::len);
                if copied_bytes > MAX_BYTES { return Err("Copies trop volumineuses (64 Mio maximum)".into()); }
                for version in &mut note.versions {
                    version.content = remap_content(&version.content, &mapping, &titles)?.0; copied_bytes += version.content.len();
                    if copied_bytes > MAX_BYTES { return Err("Copies trop volumineuses (64 Mio maximum)".into()); }
                }
            }
            note.profile_id = if preserve_profiles {
                if let Some(id) = profile_ids.get(&original.profile_id) { id.clone() } else {
                    let profile = plan.profiles.iter().find(|p| p.id == original.profile_id).ok_or("Profil d'origine absent")?;
                    let id = if db.profiles.iter().any(|p| p.id == profile.id) { profile.id.clone() }
                        else if let Some(existing) = db.profiles.iter().find(|p| p.name.to_lowercase() == profile.name.to_lowercase()) { existing.id.clone() }
                        else { db.profiles.push(profile.clone()); profile.id.clone() };
                    profile_ids.insert(original.profile_id.clone(), id.clone()); id
                }
            } else { destination.clone().unwrap() };
            note.revision = 0; first_profile.get_or_insert_with(|| note.profile_id.clone()); db.notes.push(note); added += 1;
        }
        Ok(ImportResult { added, skipped, profile_id: first_profile })
    })?;
    s.imports.remove(token);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{sample_note, Sandbox};

    fn seed(s: &mut Session) {
        mutate_database(s, |db| {
            db.profiles.push(Profile { id: "p".into(), name: "Perso".into(), created_at: Utc::now() });
            db.notes.push(sample_note()); Ok(())
        }).unwrap();
    }
    fn file(name: &str, data: &[u8]) -> InputFile { InputFile { name: name.into(), data: B64.encode(data) } }
    fn artifact_file(artifact: &Artifact) -> InputFile { InputFile { name: artifact.name.clone(), data: artifact.data.clone() } }

    #[test]
    fn markdown_package_roundtrip_preserves_metadata_links_and_binary_attachment() {
        let from = Sandbox::new(); let mut source = from.session(); seed(&mut source);
        source.db.notes[0].title = "Titre / Unicode 🌍".into();
        source.db.notes[0].tags = vec!["travail".into()];
        source.db.notes[0].attachment_data = Some(B64.encode([0, 1, 255]));
        source.db.notes[0].versions.push(NoteVersion { title: "ancien".into(), content: "avant".into(), saved_at: Utc::now() });
        let artifact = export(&source.db, &["n".into()], "markdown", None).unwrap();
        let entries = read_zip(&B64.decode(&artifact.data).unwrap()).unwrap();
        assert!(entries.contains_key("manifest.json"));
        assert!(entries.keys().all(|p| !p.contains("..")));
        let to = Sandbox::new(); let mut destination = to.session();
        let preview = prepare(&mut destination, vec![artifact_file(&artifact)], None).unwrap();
        assert_eq!(preview.attachments_count, 1);
        assert_eq!(commit(&mut destination, &preview.token, None, true, false).unwrap().added, 1);
        let imported = &destination.db.notes[0];
        assert_eq!(imported.title, source.db.notes[0].title);
        assert_eq!(imported.content, source.db.notes[0].content);
        assert_eq!(imported.tags, ["travail"]);
        assert_eq!(imported.versions[0].content, "avant");
        assert_eq!(B64.decode(imported.attachment_data.as_ref().unwrap()).unwrap(), [0, 1, 255]);
        let again = prepare(&mut destination, vec![artifact_file(&artifact)], None).unwrap();
        assert_eq!(again.duplicates, 1);
        assert_eq!(commit(&mut destination, &again.token, None, true, false).unwrap().skipped, 1);
        assert_eq!(destination.db.notes.len(), 1);
    }

    #[test]
    fn raw_markdown_links_and_resources_are_imported_without_fetching_external_urls() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session();
        let markdown = b"---\ntitle: A\ntags: [travail, lecture]\n---\n# A\n[Vers B](b.md)\n![Image](media/pic.bin)\n[Site](https://example.test)\n```\n[code](b.md)\n```";
        let preview = prepare(&mut s, vec![file("Root/a.md", markdown), file("Root/b.md", b"# B"), file("Root/media/pic.bin", &[0, 255]), file("Root/unused.bin", b"unused")], None).unwrap();
        assert_eq!(preview.ignored_files, 1);
        commit(&mut s, &preview.token, None, true, false).unwrap();
        let a = s.db.notes.iter().find(|n| n.title == "A").unwrap();
        let b = s.db.notes.iter().find(|n| n.title == "B").unwrap();
        assert!(a.content.contains(&format!("[[{}|Vers B]]", b.id)));
        assert!(a.content.contains("[code](b.md)"));
        assert!(a.content.contains("https://example.test"));
        assert!(a.content.starts_with("---"));
        assert_eq!(a.tags, ["travail", "lecture"]);
        assert_eq!(B64.decode(a.attachment_data.as_ref().unwrap()).unwrap(), [0, 255]);
    }

    #[test]
    fn multiple_resources_are_retained_as_a_downloadable_zip_attachment() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session();
        let preview = prepare(&mut s, vec![file("note.md", b"![a](a.bin) ![b](b.bin)"), file("a.bin", b"A"), file("b.bin", b"B")], None).unwrap();
        assert_eq!(preview.resource_bundles, 1);
        commit(&mut s, &preview.token, None, true, false).unwrap();
        let bundle = read_zip(&B64.decode(s.db.notes[0].attachment_data.as_ref().unwrap()).unwrap()).unwrap();
        assert_eq!(bundle["a.bin"], b"A"); assert_eq!(bundle["b.bin"], b"B");
    }

    #[test]
    fn copies_remap_internal_ids_and_highlight_offsets_without_overwriting_originals() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session(); seed(&mut s);
        s.db.notes[0].content = "[[target|Cible]] MARK".into();
        let start = s.db.notes[0].content.find("MARK").unwrap();
        s.db.notes[0].highlights.push(Highlight { id: "h".into(), start, end: start + 4, color: "yellow".into() });
        let mut target = sample_note(); target.id = "target".into(); target.title = "Cible".into(); s.db.notes.push(target);
        let artifact = export(&s.db, &["n".into(), "target".into()], "markdown", None).unwrap();
        let preview = prepare(&mut s, vec![artifact_file(&artifact)], None).unwrap();
        commit(&mut s, &preview.token, Some("p".into()), false, true).unwrap();
        assert_eq!(s.db.notes.len(), 4);
        assert_eq!(s.db.notes[0].content, "[[target|Cible]] MARK");
        let new_target = s.db.notes.iter().find(|n| n.title == "Cible" && n.id != "target").unwrap();
        let copy = s.db.notes.iter().find(|n| n.id != "n" && n.content.ends_with(" MARK")).unwrap();
        assert!(copy.content.contains(&new_target.id));
        assert_eq!(copy.highlights[0].start, copy.content.find("MARK").unwrap());
    }

    #[test]
    fn edited_markdown_is_imported_and_original_content_is_kept_as_a_version() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session(); seed(&mut s);
        s.db.notes[0].content = "original".into();
        let artifact = export(&s.db, &["n".into()], "markdown", None).unwrap();
        let mut entries = read_zip(&B64.decode(&artifact.data).unwrap()).unwrap();
        let path = entries.keys().find(|p| p.ends_with(".md")).unwrap().clone();
        let edited = String::from_utf8(entries[&path].clone()).unwrap().replace("original", "édité");
        entries.insert(path, edited.into_bytes());
        let zip = write_zip(&entries.into_iter().collect::<Vec<_>>()).unwrap();
        let preview = prepare(&mut s, vec![file("edited.zip", &zip)], None).unwrap();
        assert_eq!(preview.edited_files, 1);
        commit(&mut s, &preview.token, Some("p".into()), false, true).unwrap();
        let note = s.db.notes.last().unwrap();
        assert_eq!(note.content, "édité"); assert_eq!(note.versions.last().unwrap().content, "original");
    }

    #[test]
    fn invalid_paths_and_broken_packages_never_mutate_the_vault() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session(); seed(&mut s);
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("../escape.md", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap(); zip.write_all(b"bad").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(prepare(&mut s, vec![file("bad.zip", &bytes)], None).is_err());
        assert!(prepare(&mut s, vec![file("/absolute.md", b"bad")], None).is_err());
        assert!(prepare(&mut s, vec![file("a.md", &[255])], None).is_err());
        let artifact = export(&s.db, &["n".into()], "markdown", None).unwrap();
        let mut entries = read_zip(&B64.decode(&artifact.data).unwrap()).unwrap();
        entries.retain(|p, _| !p.starts_with("attachments/"));
        let bytes = write_zip(&entries.into_iter().collect::<Vec<_>>()).unwrap();
        assert!(prepare(&mut s, vec![file("missing.zip", &bytes)], None).is_err());
        assert_eq!(s.db.notes.len(), 1); assert!(s.imports.is_empty());
    }

    #[test]
    fn failed_persistence_keeps_the_validated_plan_available_for_retry() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session();
        let preview = prepare(&mut s, vec![file("note.txt", b"contenu")], None).unwrap();
        let original = std::fs::read(&s.vault_path).unwrap(); std::fs::remove_file(&s.vault_path).unwrap();
        assert!(commit(&mut s, &preview.token, None, true, false).is_err());
        assert!(s.db.notes.is_empty()); assert!(s.imports.contains_key(&preview.token));
        crate::atomic_write(&s.vault_path, &original).unwrap();
        assert_eq!(commit(&mut s, &preview.token, None, true, false).unwrap().added, 1);
        assert!(commit(&mut s, &preview.token, None, true, false).is_err());
    }

    #[test]
    fn encrypted_transfer_preserves_individual_protection_and_rejects_wrong_passwords() {
        let from = Sandbox::new(); let mut source = from.session(); seed(&mut source);
        let mut note = sample_note(); note.id = "locked".into(); note.title = "Secret individuel".into();
        let salt = [4; 16]; let key = crypto::derive_key("note-password", &salt).unwrap(); crate::protect_note(&mut note, &key, &salt).unwrap();
        let ciphertext = note.locked_payload.clone(); source.db.notes.push(note);
        assert!(export(&source.db, &["locked".into()], "markdown", None).is_err());
        let artifact = export(&source.db, &["n".into(), "locked".into()], "encrypted", Some("archive-password".into())).unwrap();
        let to = Sandbox::new(); let mut destination = to.session(); destination.key = [9; 32];
        assert!(prepare(&mut destination, vec![artifact_file(&artifact)], Some("wrong".into())).is_err());
        assert!(destination.db.notes.is_empty());
        let preview = prepare(&mut destination, vec![artifact_file(&artifact)], Some("archive-password".into())).unwrap();
        assert_eq!(preview.protected_count, 1);
        assert!(commit(&mut destination, &preview.token, None, true, true).is_err());
        commit(&mut destination, &preview.token, None, true, false).unwrap();
        let imported = destination.db.notes.iter().find(|n| n.id == "locked").unwrap();
        assert_eq!(imported.locked_payload, ciphertext);
        assert_eq!(crate::decrypt_note_with_key(imported, &key).unwrap().title, "Secret individuel");
    }

    #[test]
    fn text_exports_roundtrip_and_markdown_code_examples_remain_verbatim() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session(); seed(&mut s);
        s.db.notes[0].content = "[[n|Lien]]\n`[[n|code]]`\n```\n[[n|bloc]]\n```".into();
        let md = export(&s.db, &["n".into()], "markdown", None).unwrap();
        let entries = read_zip(&B64.decode(md.data).unwrap()).unwrap();
        let body = std::str::from_utf8(entries.iter().find(|(p, _)| p.ends_with(".md")).unwrap().1).unwrap();
        assert!(body.contains("[Lien](0001-")); assert!(body.contains("`[[n|code]]`")); assert!(body.contains("[[n|bloc]]"));
        let text = export(&s.db, &["n".into()], "text", None).unwrap();
        let preview = prepare(&mut s, vec![artifact_file(&text)], None).unwrap();
        commit(&mut s, &preview.token, Some("p".into()), false, true).unwrap();
        assert!(s.db.notes.last().unwrap().content.contains("code"));
    }

    #[test]
    fn symlinks_and_invalid_envelope_parameters_are_rejected_before_import() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session();
        let mut bytes = write_zip(&[("note.md".into(), b"contenu".to_vec())]).unwrap();
        let central = bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bytes[central + 5] = 3;
        bytes[central + 38..central + 42].copy_from_slice(&(0o120777u32 << 16).to_le_bytes());
        assert!(prepare(&mut s, vec![file("symlink.zip", &bytes)], None).is_err());
        let envelope = Envelope { format: ENCRYPTED_FORMAT.into(), version: 1, algorithm: "argon2id-aes256gcm-v1".into(), salt: B64.encode([0; 16]), nonce: B64.encode([0; 3]), ciphertext: B64.encode([0; 16]) };
        assert!(prepare(&mut s, vec![file("invalid.snotetransfer", &serde_json::to_vec(&envelope).unwrap())], None).is_err());
        assert!(s.db.notes.is_empty()); assert!(s.imports.is_empty());
    }

    #[test]
    fn wrapped_packages_keep_their_manifest_and_duplicate_source_ids_are_rejected() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session(); seed(&mut s);
        let artifact = export(&s.db, &["n".into()], "markdown", None).unwrap();
        let entries = read_zip(&B64.decode(artifact.data).unwrap()).unwrap();
        let mut wrapped: Vec<_> = entries.into_iter().map(|(p, data)| (format!("Export/{p}"), data)).collect();
        wrapped.push(("__MACOSX/._note.md".into(), vec![255]));
        let archive = write_zip(&wrapped).unwrap();
        let preview = prepare(&mut s, vec![file("wrapped.zip", &archive)], None).unwrap();
        assert_eq!(preview.kind, "markdown-package"); assert_eq!(preview.ignored_files, 1);
        let duplicate = b"---\nsecurenotes_id: same\n---\nbody";
        assert!(prepare(&mut s, vec![file("a.md", duplicate), file("b.md", duplicate)], None).is_err());
        assert!(s.imports.contains_key(&preview.token));
        assert_eq!(s.db.notes.len(), 1);
    }

    #[test]
    fn native_directory_import_preserves_subfolders_skips_symlinks_and_checks_session_identity() {
        let sandbox = Sandbox::new(); let mut s = sandbox.session();
        let root = sandbox.0.join("selected"); std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("notes/a.md"), "# A\n[B](b.md)").unwrap();
        std::fs::write(root.join("notes/b.md"), "# B").unwrap();
        #[cfg(unix)]
        {
            std::fs::write(sandbox.0.join("outside.md"), "outside secret").unwrap();
            std::os::unix::fs::symlink(sandbox.0.join("outside.md"), root.join("escape.md")).unwrap();
        }
        let plan = directory_plan(&root).unwrap();
        assert_eq!(plan.notes.len(), 2); assert_eq!(plan.profiles[0].name, "notes");
        #[cfg(unix)]
        assert_eq!(plan.ignored_files, 1);
        let old_id = s.id.clone(); s.id = Uuid::new_v4().to_string();
        assert!(stage_directory(&mut s, &old_id, plan.clone(), "selected".into()).is_err());
        assert!(s.imports.is_empty());
        let id = s.id.clone();
        let preview = stage_directory(&mut s, &id, plan, "selected".into()).unwrap();
        assert_eq!(preview.source.as_deref(), Some("selected"));
        commit(&mut s, &preview.token, None, true, false).unwrap();
        assert_eq!(s.db.notes.len(), 2);
    }
}
