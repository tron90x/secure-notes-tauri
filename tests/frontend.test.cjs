const { test: nodeTest } = require('node:test');
const test = (name, fn) => nodeTest(name, { timeout: 3000 }, fn);
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { frontend, editorView } = require('./helpers/frontend.cjs');

test('user text is escaped in cards, profiles, trash, versions and attachments', async () => {
  const { context, get, run } = frontend();
  const payload = '<img src=x onerror="bad()">';
  const note = { id: 'n', profile_id: 'p', title: payload, content: payload, attachment_name: payload,
    created_at: '2026-10-25T12:00:00Z', modified_history: [], versions: [{ content: payload, saved_at: '2026-10-25T12:00:00Z' }] };
  context.fixture = note;
  context.invoke = async cmd => cmd === 'list_profiles' ? [{ id: 'p', name: payload }] : cmd === 'list_trash' ? [note] : [];
  context.renderNotes([note]);
  assert(!get('notesGrid').children[0].innerHTML.includes('<img'));
  await context.loadProfiles();
  assert(!get('profileList').children[0].innerHTML.includes('<img'));
  await context.loadTrash();
  assert(!get('notesGrid').children.at(-1).innerHTML.includes('<img'));
  run('viewingNote = fixture; renderVersions();');
  assert(!get('vVersions').children[0].innerHTML.includes('<img'));
  await context.viewNote(note);
  assert(!get('vAttach').innerHTML.includes('<img'));
});

test('wikilinks are escaped once and preserve source selection offsets', () => {
  const { context } = frontend();
  const rendered = context.renderHighlighted('[[A&B]] puis 🔐', []);
  assert(rendered.includes('data-ref="A&amp;B"'));
  assert(!rendered.includes('&amp;amp;'));
  assert(rendered.includes('>[[A&amp;B]]</span>'));
  const marked = context.renderHighlighted('🔐 texte', [{ start: 2, end: 7, color: '" onclick="bad()', id: '" onclick="bad()' }]);
  assert(!marked.includes(' onclick="bad()'));
});

test('date range includes the entire end day using an exclusive next-day boundary', () => {
  const { context, get } = frontend();
  get('dfrom').value = '2026-12-31';
  get('dto').value = '2026-12-31';
  const params = context.searchParams();
  assert.equal(params.date_from, '2026-12-31T00:00:00.000Z');
  assert.equal(params.date_to, '2027-01-01T00:00:00.000Z');
});

test('reset forms discard passwords, attachments and stale file-reader results', async () => {
  const { context, get, run } = frontend();
  const readers = [];
  context.FileReader = class { constructor() { readers.push(this); } readAsDataURL() {} };
  run('currentProfile = { id: "p" }; pendingFile = { name: "old.txt", data: "YQ==" };');
  get('notePass').value = 'old-password';
  get('noteWallpaper').value = 'sunset';
  await get('newNoteBtn').onclick();
  assert.equal(get('notePass').value, '');
  assert.equal(get('noteWallpaper').value, 'aurora');
  assert.equal(run('pendingFile'), null);
  get('noteFile').onchange({ target: { files: [{ name: 'stale.txt' }] } });
  assert.equal(get('saveNoteBtn').disabled, true);
  context.resetNoteForm();
  readers[0].result = 'data:text/plain;base64,YQ==';
  readers[0].onload();
  assert.equal(run('pendingFile'), null);
  assert.equal(get('fileHint').textContent, '');
});

test('editing transmits attachment replacement or removal and blocks duplicate saves', async () => {
  const { context, get, run } = frontend();
  const calls = [];
  let resolveSave;
  context.invoke = async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === 'open_note_editor') return editorView({ note_id: 'n' });
    if (cmd === 'save_note_editor') {
      await new Promise(resolve => { resolveSave = resolve; });
      return { note_id: 'n', protected: false, committed: true };
    }
    return [];
  };
  run('currentProfile = { id: "p" };');
  await context.openEditor({ note_id: 'n' });
  run('pendingFile = { name: "new.txt", data: "YQ==" }; markNoteDirty();');
  const save = get('saveNoteBtn').onclick();
  await get('saveNoteBtn').onclick();
  assert.equal(calls.filter(([cmd]) => cmd === 'save_note_editor').length, 1);
  const replacement = calls.find(([cmd]) => cmd === 'save_note_editor')[1].params.content;
  assert.equal(replacement.attachment_name, 'new.txt');
  assert.equal(replacement.attachment_data, 'YQ==');
  resolveSave();
  await save;
  await context.openEditor({ note_id: 'n' });
  get('removeAttachment').checked = true;
  context.markNoteDirty();
  const remove = get('saveNoteBtn').onclick();
  const args = calls.findLast(([cmd]) => cmd === 'save_note_editor')[1].params.content;
  assert.equal(args.attachment_data, null);
  assert.equal(args.attachment_name, null);
  resolveSave();
  await remove;
});

test('protected note reveals payload only after authentication and exports exact attachment bytes', async () => {
  const { context, get, run } = frontend();
  let exported;
  context.URL = { createObjectURL(blob) { exported = blob; return 'blob:attachment'; }, revokeObjectURL() {} };
  context.invoke = async () => ({ title: 'secret', content: 'private content', attachment_name: '<secret>.bin', attachment_data: 'AAH/' });
  const opening = context.viewNote({ id: 'n', is_locked: true, modified_history: [] });
  get('sysInput').value = 'password';
  get('sysOk').onclick();
  await opening;
  assert.equal(get('vTitle').textContent, '🔒 secret');
  assert.equal(get('vContent').innerHTML, 'private content');
  assert.equal(get('vAttach').children[0].textContent, '📎 Télécharger : <secret>.bin');
  get('vAttach').children[0].onclick();
  assert.deepEqual([...new Uint8Array(await exported.arrayBuffer())], [0, 1, 255]);
  get('closeViewBtn').onclick();
  assert.equal(run('viewingPlaintext'), '');
  assert.equal(run('viewingNote'), null);
  assert.equal(get('vAttach').children.length, 0);
  assert.equal(get('focusContent').innerHTML, '');
});

test('activity is forwarded with throttling and note-lock errors do not end the vault session', async () => {
  const { context, get } = frontend();
  let time = 10000;
  context.Date = { now: () => time };
  const calls = [];
  context.invoke = async cmd => { calls.push(cmd); };
  get('mainView').classList.remove('hidden');
  await context.recordActivity();
  await context.recordActivity();
  assert.deepEqual(calls, ['touch_session']);
  time += 5000;
  await context.recordActivity();
  assert.deepEqual(calls, ['touch_session', 'touch_session']);
  assert.equal(await context.sessionError('Note verrouillée : tags impossibles'), false);
});

test('locking conceals open modals and clears plaintext before the backend responds', async () => {
  const { context, get, run } = frontend();
  let finishLock;
  context.invoke = () => new Promise(resolve => { finishLock = resolve; });
  run('viewingPlaintext = "secret"; currentNotes = [{ content: "secret" }];');
  get('noteContent').value = 'draft';
  for (const id of ['mainView', 'noteModal', 'dashModal', 'focusModal', 'sysModal']) get(id).classList.remove('hidden');
  const locking = context.lockAndReload();
  for (const id of ['mainView', 'noteModal', 'dashModal', 'focusModal', 'sysModal']) assert(get(id).classList.contains('hidden'));
  assert.equal(get('noteContent').value, '');
  assert.equal(run('viewingPlaintext'), '');
  assert.equal(run('currentNotes.length'), 0);
  await Promise.resolve();
  finishLock();
  await locking;
});

test('new notes autosave encrypted drafts after a pause and publish only on explicit save', async () => {
  const { context, get, tick } = frontend();
  const requests = [];
  context.invoke = async (cmd, args) => {
    if (cmd === 'open_note_editor') return editorView();
    if (cmd === 'save_note_editor') {
      requests.push(args.params);
      return { note_id: args.params.commit ? 'new-note' : null, protected: false, committed: args.params.commit };
    }
    return [];
  };
  await context.openEditor({ profile_id: 'p' });
  get('noteContent').value = 'Un brouillon à conserver';
  get('noteContent').oninput();
  get('noteTitle').value = 'Titre';
  get('noteTitle').oninput();
  assert.equal(requests.length, 0);
  await tick(1200);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].commit, false);
  assert.equal(requests[0].content.content, 'Un brouillon à conserver');
  assert(get('saveStatus').textContent.includes('Brouillon chiffré'));
  await get('saveNoteBtn').onclick();
  assert.equal(requests.length, 2);
  assert.equal(requests[1].commit, true);
  assert(get('noteModal').classList.contains('hidden'));
});

test('autosave serializes in-flight writes and preserves text typed during saving', async () => {
  const { context, get, tick, run } = frontend();
  const requests = [];
  let finishFirst;
  context.invoke = async (cmd, args) => {
    if (cmd === 'open_note_editor') return editorView({ note_id: 'n' });
    if (cmd === 'save_note_editor') {
      requests.push(args.params);
      if (requests.length === 1) await new Promise(resolve => { finishFirst = resolve; });
      return { note_id: 'n', protected: false, committed: true };
    }
    return [];
  };
  await context.openEditor({ note_id: 'n' });
  get('noteContent').value = 'Première frappe';
  get('noteContent').oninput();
  const first = context.flushEditor(false);
  get('noteContent').value = 'Deuxième frappe pendant la sauvegarde';
  get('noteContent').oninput();
  assert.equal(requests.length, 1);
  finishFirst();
  await first;
  assert.equal(get('noteContent').value, 'Deuxième frappe pendant la sauvegarde');
  assert(run('editorChange > editorSaved'));
  await tick(1200);
  assert.equal(requests.length, 2);
  assert.equal(requests[1].content.content, 'Deuxième frappe pendant la sauvegarde');
  assert.equal(requests[1].commit, true);
  assert.equal(run('editorChange'), run('editorSaved'));
});

test('failed autosave retains input and supports retry without closing the editor', async () => {
  const { context, get, tick, run } = frontend();
  let fail = true;
  context.invoke = async cmd => {
    if (cmd === 'open_note_editor') return editorView({ note_id: 'n' });
    if (cmd === 'save_note_editor') {
      if (fail) throw 'Disque indisponible';
      return { note_id: 'n', protected: false, committed: true };
    }
    return [];
  };
  await context.openEditor({ note_id: 'n' });
  get('noteContent').value = 'Texte à ne pas perdre';
  get('noteContent').oninput();
  await tick(1200);
  assert(get('saveStatus').textContent.includes('Disque indisponible'));
  assert.equal(get('noteContent').value, 'Texte à ne pas perdre');
  assert(run('editorChange > editorSaved'));
  assert(!get('noteModal').classList.contains('hidden'));
  fail = false;
  await get('saveNoteBtn').onclick();
  assert(get('noteModal').classList.contains('hidden'));
});

test('password changes are validated then cleared while later autosaves use the editor token', async () => {
  const { context, get, tick } = frontend();
  const requests = [];
  const storage = [];
  context.localStorage.setItem = (...args) => storage.push(args);
  context.invoke = async (cmd, args) => {
    if (cmd === 'open_note_editor') return editorView({ note_id: 'n', protected: true });
    if (cmd === 'save_note_editor') {
      requests.push(args.params);
      return { note_id: 'n', protected: true, committed: true };
    }
    return [];
  };
  await context.openEditor({ note_id: 'n', password: 'initial' });
  get('noteProtection').value = 'set';
  get('noteProtection').onchange();
  get('notePass').value = 'abc';
  get('notePass').oninput();
  await tick(1200);
  assert.equal(requests.length, 0);
  assert(get('saveStatus').textContent.includes('4 caractères'));
  get('notePass').value = 'nouveau-secret';
  get('notePass').oninput();
  await tick(1200);
  assert.equal(requests[0].new_password, 'nouveau-secret');
  assert.equal(get('notePass').value, '');
  assert.equal(get('noteProtection').value, 'keep');
  get('noteContent').value = 'Suite';
  get('noteContent').oninput();
  await tick(1200);
  assert.equal(requests[1].new_password, null);
  assert.equal(requests[1].token, 'editor-token');
  assert.deepEqual(storage, []);
  await get('cancelNoteBtn').onclick();
  assert.equal(get('noteContent').value, '');
});

test('protected editing authenticates and draft recovery can request a new independent note', async () => {
  const { context, get, run } = frontend();
  const opens = [];
  context.invoke = async (cmd, args) => {
    if (cmd === 'open_note_editor') { opens.push(args.params); return editorView({ protected: true, note_id: args.params.note_id ?? null }); }
    if (cmd === 'list_note_drafts') return [{ id: 'recover', profile_id: 'p', protected: true, updated_at: '2026-10-25T12:00:00Z' }];
    return [];
  };
  run('viewingNote = { id: "n", is_locked: true }; profiles = [{ id: "p", name: "Perso" }];');
  const editing = get('editViewBtn').onclick();
  get('sysInput').value = 'mot-de-passe';
  get('sysOk').onclick();
  await editing;
  assert.equal(opens[0].password, 'mot-de-passe');
  assert.equal(get('notePass').value, '');
  await get('cancelNoteBtn').onclick();
  await context.openDrafts();
  const recovering = context.recoverDraft('recover', true);
  get('sysInput').value = 'mot-du-brouillon';
  get('sysOk').onclick();
  await recovering;
  assert.equal(opens[1].draft_id, 'recover');
  assert.equal(opens[1].recover_as_new, true);
  assert.equal(opens[1].password, 'mot-du-brouillon');
  assert(!get('noteModal').classList.contains('hidden'));
});

test('closing the application waits for a recoverable draft and preserves the window on write failure', async () => {
  const { context, get } = frontend();
  let onClose, closed = 0, fail = true;
  const saves = [];
  context.window.__TAURI__ = { window: { getCurrentWindow: () => ({
    async onCloseRequested(callback) { onClose = callback; }, async destroy() { closed++; }
  }) } };
  context.invoke = async (cmd, args) => {
    if (cmd === 'get_auto_lock_timeout') return 300;
    if (cmd === 'open_note_editor') return editorView();
    if (cmd === 'save_note_editor') {
      saves.push(args.params);
      if (fail) throw 'Disque plein';
      return { note_id: null, protected: false, committed: false };
    }
    return [];
  };
  await context.enterMain();
  await context.openEditor({ profile_id: 'p' });
  get('noteContent').value = 'Dernière saisie avant fermeture';
  get('noteContent').oninput();
  let prevented = 0;
  await onClose({ preventDefault() { prevented++; } });
  assert.equal(closed, 0);
  assert.equal(get('noteContent').value, 'Dernière saisie avant fermeture');
  fail = false;
  await onClose({ preventDefault() { prevented++; } });
  assert.equal(prevented, 2);
  assert.equal(closed, 1);
  assert.equal(saves.at(-1).commit, false);
  assert.equal(saves.at(-1).content.content, 'Dernière saisie avant fermeture');
});

test('locking captures the latest draft before clearing the editor and releasing its key', async () => {
  const { context, get } = frontend();
  const calls = [];
  let finishDraft;
  context.invoke = async (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === 'open_note_editor') return editorView({ note_id: 'n', protected: true });
    if (cmd === 'save_note_editor') return new Promise(resolve => { finishDraft = resolve; });
    return [];
  };
  await context.openEditor({ note_id: 'n', password: 'secret' });
  get('noteContent').value = 'Derniers caractères';
  get('noteContent').oninput();
  const locking = context.lockAndReload();
  assert.equal(get('noteContent').value, '');
  const snapshot = calls.find(([cmd]) => cmd === 'save_note_editor')[1].params;
  assert.equal(snapshot.content.content, 'Derniers caractères');
  assert.equal(snapshot.commit, false);
  assert(!calls.some(([cmd]) => cmd === 'close_note_editor'));
  finishDraft({ note_id: 'n', protected: true, committed: false });
  await locking;
  assert.equal(calls.at(-1)[0], 'lock_vault');
});

test('HTML references, translation keys and registered Tauri commands stay consistent', () => {
  const { run } = frontend();
  const html = fs.readFileSync(path.join(__dirname, '../frontend/index.html'), 'utf8');
  const app = ['app.js', 'commands.js', 'organization.js', 'transfer.js'].map(name => fs.readFileSync(path.join(__dirname, '../frontend', name), 'utf8')).join('\n');
  const rust = fs.readFileSync(path.join(__dirname, '../src-tauri/src/main.rs'), 'utf8');
  const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(m => m[1]);
  assert.equal(ids.length, new Set(ids).size);
  const dynamic = ['decoyPass', 'decoyBox', 'rmDecoy', 'setDecoy'];
  for (const [, id] of app.matchAll(/(?:\$|\bget)\("([^"]+)"\)/g)) assert(ids.includes(id) || dynamic.includes(id), `Missing element: ${id}`);
  for (const [, key] of html.matchAll(/data-i18n(?:-ph|-title|-aria)?="([^"]+)"/g)) {
    for (const lang of ['fr', 'kab']) assert(run(`STRINGS[${JSON.stringify(lang)}][${JSON.stringify(key)}] !== undefined`), `${lang}: ${key}`);
  }
  const registered = rust.slice(rust.indexOf('.invoke_handler('), rust.indexOf('.run(tauri::generate_context!())'));
  for (const [, command] of app.matchAll(/invoke\("([^"]+)"/g)) assert(registered.includes(command), `Unregistered command: ${command}`);
});
