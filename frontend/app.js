async function invoke(cmd, args = {}) {
  // Attend que le bridge Tauri soit injecté (avec withGlobalTauri:true)
  for (let i = 0; i < 50 && !(window.__TAURI__ && window.__TAURI__.core); i++)
    await new Promise(r => setTimeout(r, 100));
  if (!window.__TAURI__?.core) throw "Bridge Tauri introuvable — relancez avec `npx tauri dev` (pas en navigateur)";
  return window.__TAURI__.core.invoke(cmd, args);
}

const $ = (id) => document.getElementById(id);
let profiles = [], currentProfile = null, currentNotes = [], editingId = null, viewingNote = null, pendingFile = null;
let showingTrash = false, idleSecs = 300, idleTimer = null;
let selectedTag = null, pendingTags = [];
let decoyVisible = false; // carré leurre masqué par défaut (Ctrl+L pour l'afficher)

// --- Modale système : remplace alert / confirm / prompt natifs ---
function sysDialog({ title, msg, input = false, password = false, okText = null, showCancel = true }) {
  return new Promise((resolve) => {
    $("sysTitle").textContent = title;
    $("sysMsg").textContent = msg || "";
    const inp = $("sysInput");
    inp.classList.toggle("hidden", !input);
    if (input) { inp.value = ""; inp.type = password ? "password" : "text"; inp.placeholder = password ? "Mot de passe..." : "Votre saisie..."; }
    $("sysOk").textContent = okText ?? (typeof t === "function" ? t("ok_btn") : "OK");
    $("sysCancel").style.display = showCancel ? "" : "none";
    $("sysModal").classList.remove("hidden");
    if (input) setTimeout(() => inp.focus(), 60);
    const done = (val) => {
      $("sysModal").classList.add("hidden");
      $("sysOk").onclick = $("sysCancel").onclick = inp.onkeydown = null;
      resolve(val);
    };
    $("sysOk").onclick = () => done(input ? { ok: true, value: inp.value } : { ok: true });
    $("sysCancel").onclick = () => done({ ok: false });
    inp.onkeydown = (e) => { if (e.key === "Enter") $("sysOk").click(); };
  });
}
const sysAlert = (msg, title = "ℹ️ Info") => sysDialog({ title, msg, showCancel: false });
const sysConfirm = (msg, title = "❓ Confirmer", okText = "Confirmer") => sysDialog({ title, msg, okText });
const sysPrompt = (msg, title = "🔒 Saisie", password = false) => sysDialog({ title, msg, input: true, password });

// Erreur de session (verrouillée / expirée) → retour à l'écran des coffres
async function sessionError(e) {
  const s = String(e && (e.message || e));
  if (s.includes("verrouillé") || s.includes("expirée") || s.includes("Session")) {
    await sysAlert(s + " — reconnectez-vous.", "🔒 Session terminée");
    location.reload();
    return true;
  }
  return false;
}

// particules
const cv = $("particles"), ctx = cv.getContext("2d");
function resize(){ cv.width = innerWidth; cv.height = innerHeight; }
addEventListener("resize", resize); resize();
const dots = Array.from({length: 90}, () => ({x: Math.random()*innerWidth, y: Math.random()*innerHeight, v: Math.random()*1.2+.2, r: Math.random()*2+.5}));
(function loop(){ ctx.clearRect(0,0,cv.width,cv.height); ctx.fillStyle = "rgba(140,180,255,.7)";
  for(const d of dots){ d.y -= d.v; if(d.y<0) d.y = innerHeight; ctx.beginPath(); ctx.arc(d.x,d.y,d.r,0,7); ctx.fill(); }
  requestAnimationFrame(loop); })();

// langue (FR / Kabyle) — appliquée dès le démarrage, mémorisée
try { setLang(localStorage.getItem("sn_lang") || "fr"); } catch (e) { setLang("fr"); }
if (!currentProfile) $("currentProfileTitle").textContent = t("select_profile");
if (!editingId) $("modalTitle").textContent = t("modal_new");
function refreshDynamicTexts(){
  if (!currentProfile && !showingTrash) $("currentProfileTitle").textContent = t("select_profile");
  if ($("noteModal").classList.contains("hidden") || !editingId) $("modalTitle").textContent = t("modal_new");
  if (typeof viewingNote !== "undefined" && viewingNote) renderVersions();
}
$("langFr").onclick = () => { setLang("fr"); refreshDynamicTexts(); };
$("langKab").onclick = () => { setLang("kab"); refreshDynamicTexts(); };

// splash -> vault (skippable au clic)
let splashDone = false;
function hideSplash(){
  if (splashDone) return; splashDone = true;
  $("splash").style.display = "none";
  $("vaultView").classList.remove("hidden");
  refreshVaults();
}
setTimeout(hideSplash, 1500);
$("splash").onclick = hideSplash;

async function refreshVaults(){
  try {
    const list = await invoke("list_vaults");
    const sel = $("vaultSelect"); sel.innerHTML = "";
    if(!list.length) sel.innerHTML = `<option value="">${t("no_vault")}</option>`;
    for(const [name, path] of list){ const o = document.createElement("option"); o.value = path; o.textContent = `📦 ${name}`; sel.appendChild(o); }
  } catch(e){ $("vaultMsg").textContent = e; }
}
// Jauge de force du mot de passe (création de base)
$("refreshVaults").onclick = refreshVaults;
$("newVaultPass").oninput = (e) => {
  const v = e.target.value;
  let score = 0;
  if (v.length >= 8) score++;
  if (v.length >= 12) score++;
  if (/[a-z]/.test(v) && /[A-Z]/.test(v)) score++;
  if (/\d/.test(v)) score++;
  if (/[^A-Za-z0-9]/.test(v)) score++;
  const pct = [0, 20, 40, 60, 80, 100][score];
  const col = ["", "#ef4444", "#f59e0b", "#eab308", "#22c55e", "#10b981"][score];
  const lbl = t("strength_levels")[score];
  $("vaultStrengthBar").style.width = pct + "%";
  $("vaultStrengthBar").style.background = col;
  $("vaultStrengthTxt").textContent = v ? `${t("strength_prefix")} ${lbl}` : "";
};

$("createVaultBtn").onclick = async () => {
  const name = $("newVaultName").value.trim(), pass = $("newVaultPass").value;
  const btn = $("createVaultBtn");
  btn.disabled = true; btn.textContent = t("create_busy");
  try { await invoke("create_vault", { name, password: pass }); $("vaultMsg").textContent = "✅ Base créée et déchiffrée !"; enterMain(); }
  catch(e){ $("vaultMsg").textContent = "❌ " + e; }
  finally { btn.disabled = false; btn.textContent = t("create_btn"); }
};
$("unlockBtn").onclick = async () => {
  const path = $("vaultSelect").value, password = $("unlockPass").value;
  if(!path) return $("vaultMsg").textContent = t("select_base");
  const btn = $("unlockBtn");
  btn.disabled = true; btn.textContent = t("unlock_busy");
  try { const name = await invoke("unlock_vault", { path, password }); $("vaultMsg").textContent = "✅ " + name + " déchiffré"; enterMain(); }
  catch(e){ $("vaultMsg").textContent = "❌ " + e; }
  finally { btn.disabled = false; btn.textContent = t("unlock_btn"); }
};

async function enterMain(){
  $("vaultView").classList.add("hidden");
  $("mainView").classList.remove("hidden");
  try {
    idleSecs = await invoke("get_auto_lock_timeout");
    $("autoLock").value = String(idleSecs);
  } catch(e) { /* backend ancien : on garde 300s */ }
  resetIdle();
  await loadProfiles();
  await refreshTrashCount();
  refreshTagBar();
}
$("lockBtn").onclick = async () => { await invoke("lock_vault"); location.reload(); };

// Verrouillage automatique après inactivité (le backend applique aussi ce délai)
function resetIdle(){
  clearTimeout(idleTimer);
  idleTimer = setTimeout(async () => {
    try { await invoke("lock_vault"); } catch(e) {}
    location.reload();
  }, idleSecs * 1000);
}
["click", "keydown"].forEach(ev => addEventListener(ev, () => { if (!$("mainView").classList.contains("hidden")) resetIdle(); }, { passive: true }));
$("autoLock").onchange = async (e) => {
  try {
    idleSecs = await invoke("set_auto_lock_timeout", { secs: Number(e.target.value) });
    e.target.value = String(idleSecs);
    resetIdle();
  } catch(err) { if (!await sessionError(err)) sysAlert("❌ " + err); }
};

// profils
async function loadProfiles(){
  try {
    profiles = await invoke("list_profiles");
  } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  const box = $("profileList"); box.innerHTML = "";
  for(const p of profiles){
    const d = document.createElement("div");
    d.className = "profile-item" + (currentProfile?.id === p.id ? " active" : "");
    d.innerHTML = `👤 <b>${p.name}</b><small>créé le ${new Date(p.created_at).toLocaleString()}</small> <button data-del="${p.id}" style="float:right;background:none;border:none;cursor:pointer">🗑</button>`;
    d.onclick = (e) => { if(e.target.dataset.del) return delProfile(e.target.dataset.del); currentProfile = p; exitTrashMode(); loadProfiles(); loadNotes(); };
    box.appendChild(d);
  }
}
async function delProfile(id){
  const c = await sysConfirm("Supprimer ce profil ? Ses notes seront placées dans la corbeille (récupérables).", "🗑 Supprimer le profil");
  if(!c.ok) return;
  try { await invoke("delete_profile", { id }); } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  if(currentProfile?.id===id) currentProfile=null;
  loadProfiles(); $("notesGrid").innerHTML=""; refreshTrashCount();
}
$("addProfileBtn").onclick = async () => {
  const name = $("profileName").value.trim(); if(!name) return;
  try { await invoke("create_profile", { name }); } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  $("profileName").value = ""; loadProfiles();
};

// notes
async function loadNotes(){
  if(!currentProfile) return $("currentProfileTitle").textContent = t("select_profile");
  exitTrashMode();
  $("currentProfileTitle").textContent = "📁 " + currentProfile.name;
  try {
    currentNotes = await invoke("list_notes", { profileId: currentProfile.id });
  } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  selectedTag = null;
  renderNotes(currentNotes);
  refreshTagBar();
}
function renderNotes(notes){
  const g = $("notesGrid"); g.innerHTML = ""; g.onclick = null;
  if(!notes.length) g.innerHTML = `<p class="muted">Aucune note. Créez-en une ✨</p>`;
  for(const n of notes){
    const c = document.createElement("div");
    c.className = `note-card wp-${n.wallpaper||"aurora"} ${n.glass?"glassy":""}`;
    // Notes verrouillées : titre + contenu + pièce jointe masqués dans la grille.
    const title = n.is_locked ? "🔒 Note verrouillée" : n.title;
    const prev = n.is_locked ? "Note chiffrée individuellement — cliquez pour déchiffrer" : (n.content||"").slice(0,120);
    const attach = (!n.is_locked && n.attachment_name) ? ` • 📎 ${n.attachment_name}` : (n.is_locked ? ` • 📎 pièce jointe` : "");
    c.innerHTML = `<h4>${title}</h4><p>${prev}</p>
      <div class="meta">📅 ${new Date(n.created_at).toLocaleString()}${n.modified_history.length?` • ✏️ ${n.modified_history.length} modif`:""}${attach}</div>`;
    c.onclick = () => viewNote(currentNotes.find(x => x.id === n.id) ?? n);
    g.appendChild(c);
  }
}

// --- Corbeille ---
function exitTrashMode(){
  showingTrash = false;
  $("emptyTrashBtn").classList.add("hidden");
  $("newNoteBtn").classList.remove("hidden");
}
$("showTrashBtn").onclick = () => loadTrash();
async function loadTrash(){
  showingTrash = true;
  $("emptyTrashBtn").classList.remove("hidden");
  $("newNoteBtn").classList.add("hidden");
  $("currentProfileTitle").textContent = t("trash_btn");
  let trash = [];
  try { trash = await invoke("list_trash"); }
  catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  const g = $("notesGrid"); g.innerHTML = "";
  if(!trash.length) g.innerHTML = `<p class="muted">Corbeille vide ✨</p>`;
  for(const n of trash){
    const pname = (profiles.find(p => p.id === n.profile_id)?.name) || "profil supprimé";
    const c = document.createElement("div");
    c.className = `note-card wp-${n.wallpaper||"aurora"} ${n.glass?"glassy":""}`;
    const title = n.is_locked ? "🔒 Note verrouillée" : n.title;
    c.innerHTML = `<h4>${title}</h4><p class="muted">👤 ${pname} • 🗑 ${new Date(n.trashed_at).toLocaleString()}</p>
      <div class="trash-actions"><button class="btn small secondary" data-restore="${n.id}">${esc(t("restore_btn"))}</button><button class="btn small danger" data-purge="${n.id}">✕ Détruire</button></div>`;
    g.appendChild(c);
  }
  g.onclick = async (e) => {
    const r = e.target.dataset?.restore, p = e.target.dataset?.purge;
    if (r) {
      try { await invoke("restore_note", { id: r }); } catch(err) { if (!await sessionError(err)) await sysAlert("❌ " + err); return; }
      await refreshTrashCount(); loadTrash(); loadNotesSilent();
    } else if (p) {
      const c = await sysConfirm("Destruction DÉFINITIVE de cette note ? Irréversible.", "⚠️ Détruire ?", "Détruire");
      if (!c.ok) return;
      try { await invoke("purge_note", { id: p }); } catch(err) { if (!await sessionError(err)) await sysAlert("❌ " + err); return; }
      await refreshTrashCount(); loadTrash();
    }
  };
  await refreshTrashCount();
}
// Recharge silencieusement les notes du profil (après restauration) sans quitter la corbeille
async function loadNotesSilent(){
  if(!currentProfile) return;
  try { currentNotes = await invoke("list_notes", { profileId: currentProfile.id }); } catch(e) {}
}
async function refreshTrashCount(){
  try { $("trashCount").textContent = (await invoke("list_trash")).length; }
  catch(e) {}
}
$("emptyTrashBtn").onclick = async () => {
  const c = await sysConfirm("Vider entièrement la corbeille ? Destruction DÉFINITIVE et irréversible.", "⚠️ Vider la corbeille", "Vider");
  if (!c.ok) return;
  try {
    const n = await invoke("empty_trash");
    await sysAlert(`${n} note(s) détruite(s) définitivement.`, "🗑 Corbeille vidée");
  } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  await refreshTrashCount(); loadTrash();
};
$("newNoteBtn").onclick = async () => { if(!currentProfile) return sysAlert(t("create_profile_first"), t("profile_required")); editingId=null; pendingTags=[]; $("tagInput").value=""; renderTagPills(); $("modalTitle").textContent=t("modal_new"); $("noteTitle").value=""; $("noteContent").value=""; $("noteModal").classList.remove("hidden"); };
$("cancelNoteBtn").onclick = () => $("noteModal").classList.add("hidden");
// --- Tags ---
function renderTagPills(){
  $("tagPills").innerHTML = pendingTags.map((t, i) => `<span class="tag-pill">#${esc(t)} <button data-untag="${i}" title="Retirer">×</button></span>`).join("");
}
$("tagPills").onclick = (e) => {
  const i = e.target.dataset?.untag;
  if (i !== undefined) { pendingTags.splice(Number(i), 1); renderTagPills(); }
};
$("tagInput").onkeydown = (e) => {
  if (e.key !== "Enter") return;
  e.preventDefault();
  const v = e.target.value.trim().toLowerCase().slice(0, 24);
  if (v && !pendingTags.includes(v) && pendingTags.length < 12) { pendingTags.push(v); renderTagPills(); }
  e.target.value = "";
};
// --- Barre d'insertion rapide : [[ ]], @, 📍, 💡, 📅, #lieu/, #idée/ ---
function insertIntoNote(before, after = "") {
  const ta = $("noteContent");
  ta.focus();
  const s = ta.selectionStart ?? ta.value.length, e = ta.selectionEnd ?? ta.value.length;
  const sel = ta.value.slice(s, e);
  ta.value = ta.value.slice(0, s) + before + sel + after + ta.value.slice(e);
  const pos = s + before.length + sel.length + (sel ? 0 : 0);
  // Si rien de sélectionné et insertion paire ([[ ]]) : curseur au milieu
  ta.selectionStart = ta.selectionEnd = after && !sel ? s + before.length : pos + (sel ? after.length : 0);
}
document.querySelectorAll("[data-insert]").forEach(b => {
  b.onclick = () => {
    const k = b.dataset.insert;
    if (k === "link") insertIntoNote("[[", "]]");
    else if (k === "person") insertIntoNote("@");
    else if (k === "place") insertIntoNote("📍");
    else if (k === "idea") insertIntoNote("💡");
    else if (k === "date") insertIntoNote(new Date().toISOString().slice(0, 10));
    else if (k === "lieu") insertIntoNote("#lieu/");
    else if (k === "idee") insertIntoNote("#idée/");
  };
});

async function refreshTagBar(){
  let tags = [];
  try { tags = await invoke("list_tags"); } catch(e) { return; }
  const bar = $("tagBar"); bar.innerHTML = "";
  if (selectedTag) {
    const b = document.createElement("button");
    b.className = "tag-pill active"; b.textContent = `#${selectedTag} ×`;
    b.onclick = () => { selectedTag = null; refreshTagBar(); loadNotes(); };
    bar.appendChild(b);
  }
  for (const [t, c] of tags.slice(0, 12)) {
    if (t === selectedTag) continue;
    const b = document.createElement("button");
    b.className = "tag-pill"; b.textContent = `#${t} ${c}`;
    b.onclick = () => { selectedTag = t; applyTagSearch(); };
    bar.appendChild(b);
  }
}

function searchParams(){
  const parse = (v) => v ? new Date(v + "T00:00:00Z").toISOString() : null;
  return { query: $("q").value || null, profile_id: currentProfile?.id || null, date_from: parse($("dfrom").value), date_to: parse($("dto").value), tag: selectedTag || null };
}

async function applyTagSearch(){
  refreshTagBar();
  exitTrashMode();
  let res = [];
  try { res = await invoke("search_notes", { params: searchParams() }); }
  catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  renderNotes(res);
  $("currentProfileTitle").textContent = selectedTag ? `🏷 #${selectedTag} — ${res.length} résultat(s)` : `🔍 ${res.length} résultat(s)`;
}
$("noteFile").onchange = (e) => {
  const f = e.target.files[0]; if(!f) return pendingFile=null;
  const r = new FileReader();
  r.onload = () => { pendingFile = { name: f.name, data: r.result.split(",")[1] }; $("fileHint").textContent = "📎 " + f.name; };
  r.readAsDataURL(f);
};
$("saveNoteBtn").onclick = async () => {
  const title = $("noteTitle").value, content = $("noteContent").value;
  const wallpaper = $("noteWallpaper").value, glass = $("noteGlass").checked;
  const notePassword = $("notePass").value || null;
  try {
    if(editingId){
      await invoke("update_note", { id: editingId, title, content, wallpaper, glass, tags: pendingTags });
    } else {
      await invoke("create_note", { profileId: currentProfile.id, title, content, notePassword,
        attachmentName: pendingFile?.name||null, attachmentData: pendingFile?.data||null, wallpaper, glass, tags: pendingTags });
    }
    $("noteModal").classList.add("hidden"); pendingFile=null; $("notePass").value=""; loadNotes();
  } catch(e){ if (!await sessionError(e)) await sysAlert("❌ " + e); }
};

// --- Surlignage : rendu + sélection ---
let viewingPlaintext = "";
const esc = (s) => String(s ?? "").replace(/&/g,"&amp;").replace(/</g,"&lt;").replace(/>/g,"&gt;").replace(/"/g,"&quot;");

function renderHighlighted(text, highlights) {
  const chars = [...text];
  const valid = (highlights||[]).filter(h => h.start < h.end && h.start < chars.length).sort((a,b) => a.start - b.start);
  let html = "", pos = 0;
  for (const h of valid) {
    if (h.start < pos) continue; // chevauchement : on ignore
    const end = Math.min(h.end, chars.length);
    html += esc(chars.slice(pos, h.start).join(""));
    html += `<mark class="hl hl-${h.color}" data-hl="${h.id}" title="Cliqué pour retirer ce surlignage">` + esc(chars.slice(h.start, end).join("")) + `</mark>`;
    pos = end;
  }
  html += esc(chars.slice(pos).join(""));
  // Liens [[Titre]] cliquables (les crochets ne peuvent pas apparaître dans les attributs générés)
  html = html.replace(/\[\[([^\]]{1,80})\]\]/g, (m, ref) => {
    const r = ref.trim();
    return r ? `<span class="wikilink" data-ref="${esc(r)}" title="Ouvrir « ${esc(r)} »">⟦${esc(r)}⟧</span>` : m;
  });
  return html || `<span class="muted">— vide —</span>`;
}

function syncViewingNote(updated) {
  viewingNote = updated;
  const i = currentNotes.findIndex(n => n.id === updated.id);
  if (i >= 0) currentNotes[i] = updated;
  // Re-rend les vignettes pour que leur closure n'ouvre plus l'ancienne version sans surlignages
  renderNotes(currentNotes);
}

// Offsets de la sélection en caractères (marche même avec des <mark> imbriqués)
function getSelectionOffsets(container) {
  const sel = window.getSelection();
  if (!sel.rangeCount || sel.isCollapsed) return null;
  const range = sel.getRangeAt(0);
  const pre = range.cloneRange();
  pre.selectNodeContents(container);
  pre.setEnd(range.startContainer, range.startOffset);
  const start = [...pre.toString()].length;
  const end = start + [...range.toString()].length;
  return { start, end };
}

function paintMeta(n){
  $("vMeta").innerHTML = `📅 Créée le ${new Date(n.created_at).toLocaleString()}`
    + (n.modified_history.length
      ? `<br>✏️ ${n.modified_history.length} modification(s) :<br>` + n.modified_history.map(d=>`• ${new Date(d).toLocaleString()}`).join("<br>")
      : `<br>✏️ Aucune modification`);
}

function renderVersions(){
  const box = $("vVersions"); box.innerHTML = "";
  const vers = viewingNote?.versions || [];
  $("vVerSum").textContent = `${t("versions_prev")} (${vers.length})`;
  if (!vers.length) { box.innerHTML = `<span class="muted">Aucune version — elles apparaissent ici à chaque modification.</span>`; return; }
  [...vers].map((v, i) => ({ v, i })).reverse().forEach(({ v, i }) => {
    const d = document.createElement("div");
    d.className = "ver-item";
    d.innerHTML = `<span>🕓 ${new Date(v.saved_at).toLocaleString()} — ${(v.content||"").slice(0,60)}${(v.content||"").length>60?"…":""}</span>`;
    const b = document.createElement("button");
    b.className = "btn small secondary"; b.textContent = t("restore_btn");
    b.onclick = async () => {
      const c = await sysConfirm(`Restaurer le contenu du ${new Date(v.saved_at).toLocaleString()} ? Le contenu actuel sera conservé comme version.`, "📜 Restaurer une version");
      if (!c.ok) return;
      try {
        const updated = await invoke("restore_version", { id: viewingNote.id, index: i });
        syncViewingNote(updated);
        viewingPlaintext = updated.content || "";
        $("vTitle").textContent = updated.title;
        paintMeta(updated);
        $("vContent").innerHTML = renderHighlighted(viewingPlaintext, updated.highlights);
        renderVersions();
      } catch(err) { if (!await sessionError(err)) await sysAlert("❌ " + err); }
    };
    d.appendChild(b);
    box.appendChild(d);
  });
}

async function viewNote(n){
  viewingNote = n;
  viewingPlaintext = "";
  // Titre masqué tant que la note verrouillée n'est pas déchiffrée
  $("vTitle").textContent = n.is_locked ? "🔒 Note verrouillée" : n.title;
  paintMeta(n);
  $("viewCard").className = `modal-card wp-${n.wallpaper||"aurora"} ${n.glass ? "glassy" : "solid"}`;
  renderVersions();
  if(n.is_locked){
    const r = await sysPrompt("Mot de passe individuel de cette note :", "🔒 Note verrouillée", true);
    if(!r.ok) return;
    try {
      const txt = await invoke("unlock_note_content", { id: n.id, password: r.value });
      viewingPlaintext = txt;
      $("vTitle").textContent = "🔒 " + n.title;
      $("vContent").innerHTML = esc(txt);
    }
    catch(e){ if (!await sessionError(e)) $("vContent").textContent = "❌ " + e; }
  } else {
    viewingPlaintext = n.content || "";
    $("vContent").innerHTML = renderHighlighted(viewingPlaintext, n.highlights);
  }
  $("vAttach").innerHTML = (!n.is_locked && n.attachment_name) ? `📎 Pièce jointe : <b>${n.attachment_name}</b>` : (n.is_locked ? `📎 <span class="muted">pièce jointe masquée</span>` : "");
  renderViewTags();
  $("viewModal").classList.remove("hidden");
}

function renderViewTags(){
  const tags = viewingNote?.tags || [];
  $("vTags").innerHTML = tags.map(t => `<span class="tag-pill" data-vtag="${esc(t)}">#${esc(t)}</span>`).join("");
}
$("vTags").onclick = (e) => {
  const t = e.target.closest?.("[data-vtag]")?.dataset.vtag;
  if (!t) return;
  selectedTag = t;
  $("viewModal").classList.add("hidden");
  applyTagSearch();
};

// Clic sur couleur = surligner la sélection Active
document.querySelectorAll(".hl-dot").forEach(btn => {
  btn.onclick = async () => {
    try {
      if (viewingNote?.is_locked) return sysAlert("Note verrouillée : surlignage impossible.", "🖍 Surlignage");
      const off = getSelectionOffsets($("vContent"));
      if (!off) return sysAlert("Sélectionnez d'abord un mot ou une phrase dans la note.", "🖍 Surlignage");
      const updated = await invoke("add_highlight", { id: viewingNote.id, start: off.start, end: off.end, color: btn.dataset.color });
      syncViewingNote(updated);
      viewingPlaintext = updated.content || viewingPlaintext;
      $("vContent").innerHTML = renderHighlighted(viewingPlaintext, updated.highlights);
      window.getSelection().removeAllRanges();
    } catch(e){ if (!await sessionError(e)) await sysAlert("❌ " + e); }
  };
});

// Clic sur un lien [[...]] = ouvrir la note cible
async function openWikilink(ref){
  let all = [];
  try { all = await invoke("search_notes", { params: { query: null, profile_id: null, date_from: null, date_to: null, tag: null } }); }
  catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  const key = ref.toLowerCase();
  const t = all.find(n => !n.is_locked && n.title.toLowerCase() === key)
        || all.find(n => !n.is_locked && n.id === ref);
  if (!t) return sysAlert(`Aucune note nommée « ${ref} ».`, "🔗 Lien");
  viewNote(t);
}

// Clic sur un surlignage existant = le supprimer
$("vContent").onclick = async (e) => {
  const w = e.target.closest?.(".wikilink");
  if (w) { openWikilink(w.dataset.ref); return; }
  const m = e.target.closest?.("mark.hl");
  if (!m) return;
  const c = await sysConfirm("Retirer ce surlignage ?", "🧽 Surlignage", "Retirer");
  if (!c.ok) return;
  try {
    const updated = await invoke("remove_highlight", { id: viewingNote.id, highlightId: m.dataset.hl });
    syncViewingNote(updated);
    viewingPlaintext = updated.content || viewingPlaintext;
    $("vContent").innerHTML = renderHighlighted(viewingPlaintext, updated.highlights);
  } catch(err){ if (!await sessionError(err)) await sysAlert("❌ " + err); }
};
$("closeViewBtn").onclick = () => $("viewModal").classList.add("hidden");

// Lecture agrandie : même fond + mêmes marquages, texte seul
$("expandBtn").onclick = () => {
  $("fTitle").textContent = $("vTitle").textContent;
  $("focusCard").className = $("viewCard").className + " focus-card";
  $("focusContent").innerHTML = $("vContent").innerHTML;
  $("focusModal").classList.remove("hidden");
};
$("closeFocusBtn").onclick = () => $("focusModal").classList.add("hidden");
addEventListener("keydown", (e) => { if (e.key === "Escape") { $("focusModal").classList.add("hidden"); $("dashModal").classList.add("hidden"); $("graphModal").classList.add("hidden"); $("backupModal").classList.add("hidden"); } });
// Leurre : Ctrl+L / Cmd+L affiche/masque le carré leurre (dashboard principal = leurre sinon)
addEventListener("keydown", (e) => {
  if ((e.ctrlKey || e.metaKey) && (e.key === "l" || e.key === "L")) {
    e.preventDefault();
    decoyVisible = !decoyVisible;
    const box = $("decoyBox");
    if (box) box.classList.toggle("hidden", !decoyVisible);
  }
});
$("delViewBtn").onclick = async () => {
  const c = await sysConfirm("Placer cette note dans la corbeille ? Vous pourrez la restaurer.", "🗑 Corbeille", "Mettre à la corbeille");
  if (!c.ok) return;
  try { await invoke("delete_note", { id: viewingNote.id }); }
  catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  $("viewModal").classList.add("hidden");
  await refreshTrashCount();
  if (showingTrash) loadTrash(); else loadNotes();
};
$("editViewBtn").onclick = async () => {
  if(viewingNote.is_locked) return sysAlert("Note verrouillée : modification impossible sans recréation.", "✏️ Modification");
  $("viewModal").classList.add("hidden"); editingId = viewingNote.id;
  pendingTags = [...(viewingNote.tags || [])]; $("tagInput").value = ""; renderTagPills();
  $("modalTitle").textContent = t("edit_title"); $("noteTitle").value = viewingNote.title; $("noteContent").value = viewingNote.content;
  $("noteWallpaper").value = viewingNote.wallpaper||"aurora"; $("noteGlass").checked = viewingNote.glass;
  $("noteModal").classList.remove("hidden");
};

// --- Graphe multi-couches : notes [[...]] + tags + @mentions ---
let gNodes = [], gEdges = [], gDrag = null, gMoved = false;
let gRaw = null, graphMode = "all";

$("graphBtn").onclick = async () => {
  let g;
  try { g = await invoke("get_graph"); }
  catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  gRaw = g;
  $("graphModal").classList.remove("hidden");
  applyGraphMode(graphMode);
};

function gColor(str){
  let h = 0;
  for (const ch of str) h = (h * 31 + ch.codePointAt(0)) % 360;
  return `hsl(${h},70%,60%)`;
}
// Couleur par type de nœud : note = profil, tag = orange, @ = vert,
// lieu = sarcelle, idée = jaune, date = bleu ("mention" legacy = @ vert)
function gNodeColor(n){
  const k = n.kind || "note";
  if (k === "tag") return "#f59e0b";
  if (k === "person" || k === "mention") return "#22c55e";
  if (k === "place") return "#14b8a6";
  if (k === "idea") return "#eab308";
  if (k === "date") return "#0ea5e9";
  return gColor(n.profile || n.title);
}
// Filtre selon la couche active, puis mise en page
function applyGraphMode(mode){
  // Compat : ancien bouton "mentions" -> nouvelle couche "persons"
  if (mode === "mentions") mode = "persons";
  graphMode = mode;
  document.querySelectorAll("[data-graph-mode]").forEach(b =>
    b.classList.toggle("active", b.dataset.graphMode === mode));
  if (!gRaw) return;
  const isPerson = (k) => k === "person" || k === "mention";
  const keep = (n) => {
    const k = n.kind || "note";
    // Vue "Tout" = graphe principal allégé : on exclut les dates
    // (trop de nœuds 📅, visibles uniquement en mode Dates/Ussan).
    if (mode === "all") return k !== "date";
    if (mode === "notes") return k === "note";
    if (mode === "tags") return k === "tag" || k === "note";
    if (mode === "persons") return isPerson(k) || k === "note";
    if (mode === "places") return k === "place" || k === "note";
    if (mode === "ideas") return k === "idea" || k === "note";
    if (mode === "dates") return k === "date" || k === "note";
    return true;
  };
  const nodes = gRaw.nodes.filter(keep);
  const ids = new Set(nodes.map(n => n.id));
  let edges = (gRaw.edges || []).filter(e => ids.has(e.from) && ids.has(e.to));
  if (mode === "all") edges = edges.filter(e => (e.kind || "link") !== "date");
  if (mode === "notes") edges = edges.filter(e => (e.kind || "link") === "link");
  if (mode === "tags") edges = edges.filter(e => (e.kind || "") === "tag" || e.kind === "cooc");
  if (mode === "persons") edges = edges.filter(e => isPerson(e.kind || ""));
  if (mode === "places") edges = edges.filter(e => (e.kind || "") === "place");
  if (mode === "ideas") edges = edges.filter(e => (e.kind || "") === "idea");
  if (mode === "dates") edges = edges.filter(e => (e.kind || "") === "date");
  const nn = nodes.filter(n => (n.kind || "note") === "note").length;
  const nt = nodes.filter(n => n.kind === "tag").length;
  const np = nodes.filter(n => isPerson(n.kind || "")).length;
  const npl = nodes.filter(n => n.kind === "place").length;
  const ni = nodes.filter(n => n.kind === "idea").length;
  const nd = nodes.filter(n => n.kind === "date").length;
  $("graphInfo").textContent = `${nn} note(s) • ${nt} tag(s) • ${np} @(s) • ${npl} 📍 • ${ni} 💡 • ${nd} 📅 • ${edges.length} lien(s)`;
  layoutGraph({ nodes, edges });
}
document.querySelectorAll("[data-graph-mode]").forEach(b => {
  b.onclick = () => applyGraphMode(b.dataset.graphMode);
});

function layoutGraph(g){
  const cv = $("graphCanvas"), wrap = cv.parentElement;
  const W = wrap.clientWidth, H = Math.round(innerHeight * 0.6), dpr = devicePixelRatio || 1;
  cv.width = W * dpr; cv.height = H * dpr;
  cv.style.height = H + "px";
  const byId = {};
  // Mode Dates : timeline horizontale (dates triées sur l'axe X, notes au-dessus/dessous)
  if (graphMode === "dates") {
    const dates = g.nodes.filter(n => (n.kind || "note") === "date")
      .sort((a, b) => a.id.localeCompare(b.id));
    const notes = g.nodes.filter(n => (n.kind || "note") !== "date");
    const dx = (i) => dates.length <= 1 ? W / 2 : 60 + i * (W - 120) / Math.max(1, dates.length - 1);
    dates.forEach((n, i) => {
      const node = { ...n, kind: n.kind || "note", x: dx(i), y: H / 2, r: 13 + Math.min(14, (n.links || 0) * 3), color: gNodeColor(n) };
      byId[n.id] = node;
    });
    notes.forEach((n, i) => {
      const above = i % 2 === 0;
      const x = 60 + (i / Math.max(1, notes.length - 1)) * (W - 120);
      const y = above ? H / 2 - 130 - (i % 3) * 28 : H / 2 + 130 + (i % 3) * 28;
      const node = { ...n, kind: n.kind || "note", x, y: Math.max(40, Math.min(H - 40, y)), r: 14 + Math.min(16, (n.links || 0) * 3), color: gNodeColor(n) };
      byId[n.id] = node;
    });
    gNodes = [...dates, ...notes].map(n => byId[n.id]);
    gEdges = (g.edges || []).map(e => ({ ...e, kind: e.kind || "link", from: byId[e.from], to: byId[e.to] })).filter(e => e.from && e.to);
    drawGraph();
    return;
  }
  const cx = W / 2, cy = H / 2, R = Math.min(W, H) / 2 - 60;
  gNodes = g.nodes.map((n, i) => {
    const a = g.nodes.length <= 1 ? 0 : (i / g.nodes.length) * Math.PI * 2;
    // Tags / mentions importants = plus gros
    const node = { ...n, kind: n.kind || "note", x: cx + Math.cos(a) * R, y: cy + Math.sin(a) * R, r: 14 + Math.min(16, (n.links || 0) * 3), color: gNodeColor(n) };
    byId[n.id] = node;
    return node;
  });
  gEdges = (g.edges || []).map(e => ({ ...e, kind: e.kind || "link", from: byId[e.from], to: byId[e.to] })).filter(e => e.from && e.to);
  drawGraph();
}

function drawGraph(){
  const cv = $("graphCanvas"), dpr = devicePixelRatio || 1;
  const ctx = cv.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, cv.width, cv.height);
  // Axe temporel en mode Dates
  if (typeof graphMode !== "undefined" && graphMode === "dates") {
    const W = cv.width / dpr, H = cv.height / dpr;
    ctx.strokeStyle = "rgba(14,165,233,.35)"; ctx.lineWidth = 3;
    ctx.beginPath(); ctx.moveTo(40, H / 2); ctx.lineTo(W - 40, H / 2); ctx.stroke();
  }
  for (const e of gEdges) {
    // Style d'arête par type : link = bleu, tag = orange, @ = vert,
    // lieu = sarcelle, idée = jaune, date = bleu ciel, cooc = pointillé
    if (e.kind === "cooc") { ctx.strokeStyle = "rgba(245,158,11,.5)"; ctx.setLineDash([5, 4]); ctx.lineWidth = 1 + Math.min(3, (e.weight || 1)); }
    else if (e.kind === "tag") { ctx.strokeStyle = "rgba(245,158,11,.45)"; ctx.setLineDash([]); ctx.lineWidth = 1.5; }
    else if (e.kind === "person" || e.kind === "mention") { ctx.strokeStyle = "rgba(34,197,94,.5)"; ctx.setLineDash([]); ctx.lineWidth = 1.5; }
    else if (e.kind === "place") { ctx.strokeStyle = "rgba(20,184,166,.55)"; ctx.setLineDash([]); ctx.lineWidth = 1.5; }
    else if (e.kind === "idea") { ctx.strokeStyle = "rgba(234,179,8,.55)"; ctx.setLineDash([]); ctx.lineWidth = 1.5; }
    else if (e.kind === "date") { ctx.strokeStyle = "rgba(14,165,233,.5)"; ctx.setLineDash([]); ctx.lineWidth = 1.5; }
    else { ctx.strokeStyle = "rgba(140,180,255,.4)"; ctx.setLineDash([]); ctx.lineWidth = 1.5; }
    ctx.beginPath(); ctx.moveTo(e.from.x, e.from.y); ctx.lineTo(e.to.x, e.to.y); ctx.stroke();
  }
  ctx.setLineDash([]);
  ctx.textAlign = "center";
  for (const n of gNodes) {
    const kind = n.kind || "note";
    if (kind === "date") {
      // Hexagone bleu pour 📅 dates
      ctx.beginPath(); ctx.fillStyle = n.color;
      for (let k = 0; k < 6; k++) {
        const a = Math.PI / 3 * k - Math.PI / 6;
        const px = n.x + Math.cos(a) * n.r, py = n.y + Math.sin(a) * n.r;
        k ? ctx.lineTo(px, py) : ctx.moveTo(px, py);
      }
      ctx.closePath(); ctx.fill();
      ctx.strokeStyle = "rgba(255,255,255,.8)"; ctx.stroke();
    } else if (kind === "person" || kind === "mention") {
      // Losange vert pour @personnes
      ctx.beginPath(); ctx.fillStyle = n.color;
      ctx.moveTo(n.x, n.y - n.r); ctx.lineTo(n.x + n.r, n.y); ctx.lineTo(n.x, n.y + n.r); ctx.lineTo(n.x - n.r, n.y);
      ctx.closePath(); ctx.fill();
      ctx.strokeStyle = "rgba(255,255,255,.7)"; ctx.stroke();
    } else if (kind === "place") {
      // Carré sarcelle pour 📍lieux
      const s = n.r;
      ctx.beginPath(); ctx.fillStyle = n.color;
      ctx.rect(n.x - s * 0.8, n.y - s * 0.8, s * 1.6, s * 1.6);
      ctx.fill();
      ctx.strokeStyle = "rgba(255,255,255,.8)"; ctx.stroke();
    } else if (kind === "idea") {
      // Étoile/cercle jaune pour 💡idées
      ctx.beginPath(); ctx.fillStyle = n.color;
      ctx.arc(n.x, n.y, n.r * 0.9, 0, 7); ctx.fill();
      ctx.strokeStyle = "rgba(0,0,0,.45)"; ctx.lineWidth = 2; ctx.stroke();
      ctx.lineWidth = 1.5;
    } else if (kind === "tag") {
      // Carré arrondi pour #tags
      const s = n.r;
      ctx.beginPath(); ctx.fillStyle = n.color;
      if (ctx.roundRect) ctx.roundRect(n.x - s, n.y - s * 0.7, s * 2, s * 1.4, 6);
      else ctx.rect(n.x - s, n.y - s * 0.7, s * 2, s * 1.4);
      ctx.fill();
      ctx.strokeStyle = "rgba(255,255,255,.7)"; ctx.stroke();
    } else {
      const grad = ctx.createRadialGradient(n.x - 4, n.y - 4, 2, n.x, n.y, n.r);
      grad.addColorStop(0, "#fff"); grad.addColorStop(0.25, n.color); grad.addColorStop(1, "rgba(0,0,0,.4)");
      ctx.beginPath(); ctx.fillStyle = grad;
      ctx.arc(n.x, n.y, n.r, 0, 7); ctx.fill();
    }
    ctx.fillStyle = "rgba(255,255,255,.9)"; ctx.font = "11px sans-serif";
    const label = n.title.length > 18 ? n.title.slice(0, 17) + "…" : n.title;
    ctx.fillText(label, n.x, n.y + n.r + 13);
  }
}

function gHit(x, y){
  const r = $("graphCanvas").getBoundingClientRect();
  const px = x - r.left, py = y - r.top;
  return gNodes.find(n => (n.x - px) ** 2 + (n.y - py) ** 2 <= (n.r + 6) ** 2) || null;
}

$("graphCanvas").addEventListener("mousedown", (e) => {
  const n = gHit(e.clientX, e.clientY);
  if (n) { gDrag = n; gMoved = false; $("graphCanvas").style.cursor = "grabbing"; }
});
addEventListener("mousemove", (e) => {
  const cv = $("graphCanvas");
  if ($("graphModal").classList.contains("hidden")) return;
  if (gDrag) {
    const r = cv.getBoundingClientRect();
    const nx = e.clientX - r.left, ny = e.clientY - r.top;
    if (Math.abs(nx - gDrag.x) + Math.abs(ny - gDrag.y) > 4) gMoved = true;
    gDrag.x = nx; gDrag.y = ny;
    drawGraph();
    return;
  }
  const n = gHit(e.clientX, e.clientY);
  const tip = $("graphTip");
  if (n) {
    cv.style.cursor = "pointer";
    tip.classList.remove("hidden");
    const kind = n.kind || "note";
    tip.textContent = kind === "note"
      ? `${n.title} • 👤 ${n.profile} • ${n.links} lien(s)`
      : kind === "tag"
        ? `${n.title} • 🏷 ${n.links} note(s) — cliquer pour filtrer`
        : kind === "date"
          ? `${n.title} • 📅 ${n.links} note(s) — cliquer pour filtrer ce jour`
          : (kind === "person" || kind === "mention")
            ? `${n.title} • 🧔 ${n.links} note(s) — cliquer pour chercher`
            : kind === "place"
              ? `${n.title} • 📍 ${n.links} note(s) — cliquer pour chercher`
              : `${n.title} • 💡 ${n.links} note(s) — cliquer pour chercher`;
    const r = cv.parentElement.getBoundingClientRect();
    tip.style.left = Math.min(e.clientX - r.left + 14, r.width - 270) + "px";
    tip.style.top = (e.clientY - r.top + 12) + "px";
  } else {
    cv.style.cursor = "grab";
    tip.classList.add("hidden");
  }
});
addEventListener("mouseup", async () => {
  if (gDrag && !gMoved) {
    const n = gDrag;
    const kind = n.kind || "note";
    if (kind === "tag") {
      // Clic sur #tag : ferme le graphe et filtre les notes
      const tagName = n.id.startsWith("tag:") ? n.id.slice(4) : n.title.replace(/^#/, "");
      $("graphModal").classList.add("hidden");
      selectedTag = tagName;
      try { await applyTagSearch(); } catch(e) {}
      gDrag = null;
      return;
    }
    if (kind === "person" || kind === "mention") {
      // Clic sur @personne : recherche plein-texte
      const q = n.id.startsWith("person:") ? "@" + n.id.slice(7)
        : n.id.startsWith("mention:") ? "@" + n.id.slice(8) : n.title;
      $("graphModal").classList.add("hidden");
      exitTrashMode();
      $("q").value = q;
      try {
        const res = await invoke("search_notes", { params: searchParams() });
        // search backend ne connaît pas @ : filtre côté client sur titre+contenu
        const qlow = q.toLowerCase();
        const filtered = res.filter(x => ((x.title || "") + " " + (x.content || "")).toLowerCase().includes(qlow));
        renderNotes(filtered.length ? filtered : res);
        $("currentProfileTitle").textContent = `🧔 ${q} — ${filtered.length} résultat(s)`;
      } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); }
      gDrag = null;
      return;
    }
    if (kind === "place" || kind === "idea") {
      // Clic sur 📍lieu / 💡idée : recherche du mot-clé (sans emoji) dans titre+contenu
      const raw = n.id.includes(":") ? n.id.slice(n.id.indexOf(":") + 1) : n.title;
      const isIdea = kind === "idea";
      $("graphModal").classList.add("hidden");
      exitTrashMode();
      selectedTag = null;
      $("q").value = "";
      $("dfrom").value = ""; $("dto").value = "";
      refreshTagBar();
      try {
        const all = await invoke("search_notes", { params: { query: null, profile_id: currentProfile?.id || null, date_from: null, date_to: null, tag: null } });
        const qlow = raw.toLowerCase();
        const filtered = all.filter(x => ((x.title || "") + " " + (x.content || "")).toLowerCase().includes(qlow));
        renderNotes(filtered);
        $("currentProfileTitle").textContent = `${isIdea ? "💡" : "📍"} ${raw} — ${filtered.length} résultat(s)`;
      } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); }
      gDrag = null;
      return;
    }
    if (kind === "date") {
      // Clic sur 📅 date : filtre journalier (dfrom=date, dto=lendemain car date_to est exclusif)
      const day = n.id.startsWith("date:") ? n.id.slice(5) : null;
      $("graphModal").classList.add("hidden");
      if (day) {
        exitTrashMode();
        selectedTag = null;
        $("q").value = "";
        $("dfrom").value = day;
        const next = new Date(day + "T00:00:00Z");
        next.setUTCDate(next.getUTCDate() + 1);
        $("dto").value = next.toISOString().slice(0, 10);
        refreshTagBar();
        try {
          const res = await invoke("search_notes", { params: searchParams() });
          renderNotes(res);
          $("currentProfileTitle").textContent = `📅 ${day} — ${res.length} résultat(s)`;
        } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); }
      }
      gDrag = null;
      return;
    }
    $("graphModal").classList.add("hidden");
    try {
      const all = await invoke("search_notes", { params: { query: null, profile_id: null, date_from: null, date_to: null, tag: null } });
      const full = all.find(x => x.id === n.id);
      if (full) viewNote(full);
    } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); }
  }
  gDrag = null;
  if (!$("graphModal").classList.contains("hidden")) $("graphCanvas").style.cursor = "grab";
});
$("closeGraphBtn").onclick = () => $("graphModal").classList.add("hidden");

// --- Backups ---
$("backupBtn").onclick = openBackups;
async function openBackups(){
  let list = [];
  try { list = await invoke("list_backups"); }
  catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  $("backupList").innerHTML = list.length ? list.map(b =>
    `<div class="bak-row"><span>📦 <b>${esc(b.name)}</b><br><span class="muted">${b.modified ? new Date(b.modified).toLocaleString() : ""} • ${fmtKB(b.bytes)}</span></span><button class="btn small secondary" data-restore="${esc(b.name)}">${esc(t("restore_btn"))}</button></div>`
  ).join("") : `<p class="muted">Aucune sauvegarde pour le moment.</p>`;
  $("backupModal").classList.remove("hidden");
}
$("closeBackupBtn").onclick = () => $("backupModal").classList.add("hidden");
$("doBackupBtn").onclick = async () => {
  try {
    const n = await invoke("backup_now");
    await sysAlert(`Sauvegarde ${n} créée.`, "📦 Sauvegarde");
  } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  openBackups();
};
$("backupList").onclick = async (e) => {
  const name = e.target.dataset?.restore;
  if (!name) return;
  const c = await sysConfirm("Restaurer cette sauvegarde ? L'état Actuel sera d'abord copié en sécurité, puis l'app reviendra à l'écran des coffres.", "↩ Restaurer", "Restaurer");
  if (!c.ok) return;
  try { await invoke("restore_backup", { name }); }
  catch(e2) { if (!await sessionError(e2)) await sysAlert("❌ " + e2); return; }
  try { await invoke("lock_vault"); } catch(e3) {}
  location.reload();
};

// --- Dashboard ---
const HL_PALETTE = { yellow: "#fde047", green: "#86efac", pink: "#f9a8d4", blue: "#93c5fd", orange: "#fdba74" };
const WALL_PALETTE = { aurora: "#8b5cf6", sunset: "#f59e0b", ocean: "#0ea5e9", neon: "#22c55e", sakura: "#f472b6", carbon: "#6b7280" };
const fmtKB = (b) => b > 1048576 ? (b / 1048576).toFixed(1) + " Mo" : Math.max(1, Math.round(b / 1024)) + " Ko";

function hbars(el, rows) {
  const max = Math.max(1, ...rows.map(r => r.value));
  el.innerHTML = rows.length ? rows.map(r =>
    `<div class="bar-row"><span title="${esc(r.label)}">${esc(r.label)}</span><div class="bar-track"><div class="bar-fill" style="width:${Math.round(r.value / max * 100)}%;background:${r.color || "#22d3ee"}"></div></div><b>${r.value}</b></div>`
  ).join("") : `<span class="muted">— aucune donnée —</span>`;
}

function drawDonut(cv, locked, unlocked) {
  const size = 200, dpr = devicePixelRatio || 1;
  cv.width = size * dpr; cv.height = size * dpr;
  cv.style.width = size + "px"; cv.style.height = size + "px";
  const ctx = cv.getContext("2d"); ctx.scale(dpr, dpr);
  const total = locked + unlocked;
  if (!total) {
    ctx.beginPath(); ctx.strokeStyle = "rgba(255,255,255,.15)"; ctx.lineWidth = 32;
    ctx.arc(size / 2, size / 2, size / 2 - 20, 0, 7); ctx.stroke();
  } else {
    let a = -Math.PI / 2;
    for (const [v, col] of [[locked, "#f472b6"], [unlocked, "#22d3ee"]]) {
      if (!v) continue;
      const a2 = a + (v / total) * Math.PI * 2;
      ctx.beginPath(); ctx.strokeStyle = col; ctx.lineWidth = 32;
      ctx.arc(size / 2, size / 2, size / 2 - 20, a, a2); ctx.stroke();
      a = a2;
    }
  }
  ctx.fillStyle = "#fff"; ctx.font = "bold 30px sans-serif";
  ctx.textAlign = "center"; ctx.textBaseline = "middle";
  ctx.fillText(String(total), size / 2, size / 2);
}

function drawActivity(cv, days) {
  const W = Math.max(280, cv.parentElement.clientWidth - 30), H = 170, dpr = devicePixelRatio || 1;
  cv.width = W * dpr; cv.height = H * dpr;
  cv.style.width = W + "px"; cv.style.height = H + "px";
  const ctx = cv.getContext("2d"); ctx.scale(dpr, dpr);
  const max = Math.max(1, ...days.map(d => Math.max(d.creations, d.modifications)));
  const X = (i) => 30 + i * (W - 42) / ((days.length - 1) || 1);
  const Y = (v) => H - 24 - (v / max) * (H - 52);
  ctx.font = "10px sans-serif";
  for (let g = 0; g <= 4; g++) {
    const v = Math.round(max * g / 4), y = Y(v);
    ctx.strokeStyle = "rgba(255,255,255,.12)"; ctx.beginPath();
    ctx.moveTo(30, y); ctx.lineTo(W - 10, y); ctx.stroke();
    ctx.fillStyle = "rgba(255,255,255,.6)"; ctx.fillText(String(v), 6, y + 3);
  }
  const grad = ctx.createLinearGradient(0, 0, 0, H);
  grad.addColorStop(0, "rgba(34,211,238,.45)"); grad.addColorStop(1, "rgba(34,211,238,.04)");
  ctx.beginPath();
  days.forEach((d, i) => i ? ctx.lineTo(X(i), Y(d.creations)) : ctx.moveTo(X(0), Y(d.creations)));
  ctx.strokeStyle = "#22d3ee"; ctx.lineWidth = 2; ctx.stroke();
  ctx.lineTo(X(days.length - 1), H - 24); ctx.lineTo(X(0), H - 24); ctx.closePath();
  ctx.fillStyle = grad; ctx.fill();
  ctx.beginPath();
  days.forEach((d, i) => i ? ctx.lineTo(X(i), Y(d.modifications)) : ctx.moveTo(X(0), Y(d.modifications)));
  ctx.strokeStyle = "#e879f9"; ctx.lineWidth = 2; ctx.stroke();
  ctx.fillStyle = "rgba(255,255,255,.6)";
  ctx.fillText(days[0].date, 30, H - 8);
  ctx.fillText(days[14].date, W / 2 - 12, H - 8);
  ctx.fillText(days[29].date, W - 42, H - 8);
}

$("dashBtn").onclick = async () => {
  let st;
  try { st = await invoke("get_stats"); }
  catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  $("dashKpis").innerHTML = [
    ["📝", st.notes_total, t("kpi_notes")], ["👤", st.profiles_count, t("kpi_profiles")],
    ["🔤", st.words_total, t("kpi_words")], ["📎", st.attachments_count, t("kpi_attachments")],
    ["🖍", st.highlights_total, t("kpi_highlights")], ["📜", st.versions_total, t("kpi_versions")],
    ["🗑", st.trash_count, t("kpi_trash")], ["💾", fmtKB(st.vault_bytes), t("kpi_vault")],
  ].map(([ico, v, l]) => `<div class="kpi"><b>${v}</b><span>${ico} ${l}</span></div>`).join("");
  hbars($("dashProfiles"), st.per_profile.map(p => ({ label: p.name, value: p.count, color: "#8b5cf6" })));
  drawDonut($("dashDonut"), st.locked_count, st.unlocked_count);
  $("dashDonutLegend").textContent = `🔒 ${st.locked_count} ${t("dash_locked")} • 🔓 ${st.unlocked_count} ${t("dash_free")}`;
  drawActivity($("dashActivity"), st.last_30d);
  hbars($("dashColors"), st.highlights_by_color.map(([c, v]) => ({ label: c, value: v, color: HL_PALETTE[c] || "#fff" })));
  hbars($("dashWalls"), st.wallpapers.map(([w, v]) => ({ label: w, value: v, color: WALL_PALETTE[w] || "#22d3ee" })));
  const hmax = Math.max(1, ...st.heat_84d.map(d => d.count));
  $("dashHeat").innerHTML = st.heat_84d.map(d => {
    const bg = d.count ? `rgba(34,211,238,${(0.25 + 0.75 * d.count / hmax).toFixed(2)})` : "rgba(255,255,255,.08)";
    return `<div class="cell" style="background:${bg}" title="${d.date} : ${d.count} activité(s)"></div>`;
  }).join("");
  $("dashTop").innerHTML = st.longest.length ? st.longest.map(x =>
    `<div class="top-row"><span><b>${esc(x.title)}</b> <span class="muted">• ${esc(x.profile)}</span></span><span class="muted"></span><b>${x.words} ${esc(t("kpi_words"))}</b></div>`
  ).join("") : `<span class="muted">Aucune note pour le moment.</span>`;
  const f = (d) => d ? new Date(d).toLocaleDateString() : "—";
  $("dashSec").innerHTML =
    `<div>🔐 ${t("dash_encryption")} : <b>AES-256-GCM</b></div><div>🧂 KDF : <b>Argon2id</b> (64 Mio, t=3, p=1)</div>` +
    `<div>${t("dash_sec_autolock")} : <b>${Math.round(st.auto_lock_secs / 60)} min</b> ${t("dash_sec_inactivity")}</div>` +
    `<div>${t("dash_sec_glass")} : <b>${st.glass_count}</b> ${t("dash_sec_glass_notes")} • ⬛ ${st.notes_total - st.glass_count} ${t("dash_sec_full")}</div>` +
    `<div>${t("dash_sec_attachments")} : <b>${st.attachments_count}</b> (≈ ${fmtKB(st.attachments_bytes)})</div>` +
    `<div>${t("dash_sec_oldest")} : <b>${f(st.oldest)}</b> • ${t("dash_sec_recent")} : <b>${f(st.newest)}</b></div>` +
    `<div>📦 ${t("dash_file")} : <b>${esc(st.vault_path.split(/[/\\\\]/).pop())}</b> (${fmtKB(st.vault_bytes)})</div>`;
  // Streak + objectif
  const goal = st.daily_goal_words || 0;
  const isKab = (typeof LANG !== "undefined" && LANG === "kab");
  const streakDays = isKab ? `${st.streak_current} n wass` : `${st.streak_current} j`;
  const recordLine = isKab
    ? `asekles n wass ${st.streak_best} • ${st.days_active} n wass(en) urmid(en)`
    : `record ${st.streak_best} j • ${st.days_active} j actif(s)`;
  const todayGoal = isKab
    ? `Assa-a: ${st.words_today} / ${goal} awal(en)`
    : `Aujourd'hui : ${st.words_today} / ${goal} mots`;
  const todayNoGoal = isKab
    ? `Assa-a: ${st.words_today} awal(en) - sbadu iswi ddaw-a.`
    : `Aujourd'hui : ${st.words_today} mot(s) — définis un objectif ci-dessous.`;
  $("dashStreak").innerHTML =
    `<div class="streak-big">🔥 ${streakDays}</div>` +
    `<div class="muted">${recordLine}</div>` +
    (goal
      ? `<div class="muted" style="margin-top:8px">${todayGoal}</div><div class="goal-track"><div class="goal-fill" style="width:${goal ? Math.min(100, Math.round(st.words_today / goal * 100)) : 0}%"></div></div>`
      : `<div class="muted" style="margin-top:8px">${todayNoGoal}</div>`);
  $("goalInput").value = goal || "";
  $("goalSaveBtn").onclick = async () => {
    try { await invoke("set_daily_goal", { words: Number($("goalInput").value) || 0 }); }
    catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
    $("dashBtn").onclick();
  };
  // Nuage de tags
  $("dashTags").innerHTML = st.top_tags.length ? st.top_tags.map(([t, c]) =>
    `<span class="tag-pill" data-cloudtag="${esc(t)}" style="font-size:${12 + Math.min(10, c * 2)}px">#${esc(t)} ${c}</span>`
  ).join("") : `<span class="muted">${esc(t("dash_no_tag"))}</span>`;
  // Coffre leurre : carré masqué par défaut pour que le dashboard principal
  // soit identique à celui du leurre (déniabilité). Affichage via Ctrl+L
  // (session uniquement, jamais persisté). Masqué en session leurre.
  if (!st.decoy) {
    const box = document.createElement("div");
    box.className = "decoy-box";
    box.id = "decoyBox";
    if (!decoyVisible) box.classList.add("hidden");
    box.innerHTML = st.has_decoy
      ? `🪞 <b>${esc(t("decoy_configured_title"))}</b> <span class="muted">${esc(t("decoy_desc"))}</span> <button id="rmDecoy" class="btn small danger">${esc(t("decoy_remove"))}</button>`
      : `🪞 <b>${esc(t("decoy_title"))} :</b> <span class="muted">${esc(t("decoy_desc_setup"))}</span><div class="row" style="margin-top:8px"><input id="decoyPass" type="password" placeholder="${esc(t("decoy_pass_ph"))}" /><button id="setDecoy" class="btn small secondary">${t("dash_enable")}</button></div>`;
    $("dashSec").appendChild(box);
    if (st.has_decoy) {
      $("rmDecoy").onclick = async () => {
        const c = await sysConfirm("Supprimer le coffre leurre ?", "🪞 Leurre", "Supprimer");
        if (!c.ok) return;
        try { await invoke("remove_decoy"); } catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
        $("dashBtn").onclick();
      };
    } else {
      $("setDecoy").onclick = async () => {
        try { await invoke("set_decoy", { decoyPassword: $("decoyPass").value }); }
        catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
        await sysAlert("Leurre activé. Teste-le : verrouille puis déverrouille avec le mot de passe leurre.", `🪞 ${t("decoy_title")}`);
        $("dashBtn").onclick();
      };
    }
  }
  $("dashModal").classList.remove("hidden");
};
$("dashTags").onclick = (e) => {
  const t = e.target.closest?.("[data-cloudtag]")?.dataset.cloudtag;
  if (!t) return;
  selectedTag = t;
  $("dashModal").classList.add("hidden");
  applyTagSearch();
};
$("closeDashBtn").onclick = () => $("dashModal").classList.add("hidden");

// recherche avancée
$("searchBtn").onclick = async () => {
  exitTrashMode();
  let res = [];
  try { res = await invoke("search_notes", { params: searchParams() }); }
  catch(e) { if (!await sessionError(e)) await sysAlert("❌ " + e); return; }
  // La recherche par mot-clé ignore les notes verrouillées (titre masqué, contenu chiffré).
  renderNotes(res);
  $("currentProfileTitle").textContent = selectedTag ? `🏷 #${selectedTag} — ${res.length} résultat(s)` : `🔍 ${res.length} résultat(s)`;
};
$("clearSearchBtn").onclick = () => { $("q").value=""; $("dfrom").value=""; $("dto").value=""; selectedTag=null; refreshTagBar(); loadNotes(); };
