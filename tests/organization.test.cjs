const { test: nodeTest } = require('node:test');
const test = (name, fn) => nodeTest(name, { timeout: 3000 }, fn);
const assert = require('node:assert/strict');
const { frontend, editorView } = require('./helpers/frontend.cjs');

const params = () => ({ query: 'tag:focus', profile_id: null, date_from: null, date_to: null, tag: null });
function workspace() {
  const ui = frontend(), calls = [], handlers = {};
  const profiles = [{ id: 'p', name: 'Personnel' }, { id: 'work', name: 'Travail' }];
  const folders = [{ id: 'folder', name: 'Focus', params: params(), count: 1 }];
  const source = { id: 'source', profile_id: 'p', title: 'Source', content: '[[target|Cible]] [[Absent]]', modified_history: [], versions: [], tags: ['focus'], highlights: [], created_at: '2026-10-25T12:00:00Z' };
  const target = { ...source, id: 'target', title: 'Cible', content: 'Aperçu de la cible', tags: [] };
  const summary = n => ({ id: n.id, title: n.title, profile_id: n.profile_id, profile_name: 'Personnel', is_locked: !!n.is_locked, preview: n.content });
  const noteContext = { incoming: [summary(target)], outgoing: [{ reference: 'target', label: 'Cible', status: 'resolved', targets: [summary(target)], can_create: false }, { reference: 'Absent', label: 'Absent', status: 'missing', targets: [], can_create: true }], incoming_total: 1, outgoing_total: 2 };
  ui.context.fixtureProfiles = profiles;
  ui.run('profiles = fixtureProfiles; currentProfile = profiles[0];');
  ui.get('mainView').classList.remove('hidden');
  ui.context.invoke = async (cmd, args = {}) => {
    calls.push([cmd, args]);
    if (handlers[cmd]) return handlers[cmd](args);
    if (cmd === 'list_profiles') return profiles;
    if (cmd === 'list_smart_folders') return folders;
    if (cmd === 'run_smart_folder') return { folder: folders.find(f => f.id === args.id), notes: [source] };
    if (cmd === 'save_smart_folder') {
      const saved = { id: args.id || 'created-folder', name: args.name, params: args.params, count: 1 };
      const i = folders.findIndex(f => f.id === saved.id); if (i >= 0) folders[i] = saved; else folders.push(saved);
      return saved;
    }
    if (cmd === 'get_note_context') return noteContext;
    if (cmd === 'get_note') return args.id === 'source' ? source : target;
    if (cmd === 'resolve_note_reference') return { reference: args.reference, label: args.reference, status: 'missing', targets: [], can_create: true };
    if (cmd === 'create_note_from_reference') return 'made';
    if (cmd === 'search_command_items') return { notes: [summary(target)], profiles, tags: [], folders: folders.map(({ id, name }) => ({ id, name })) };
    if (cmd === 'open_note_editor') return editorView({ token: 'editor', note_id: args.params.note_id || null, content: { ...editorView().content, title: args.params.note_id === 'made' ? 'Absent' : 'Source' } });
    if (cmd === 'save_note_editor') return { note_id: 'source', committed: args.params.commit, protected: false };
    return [];
  };
  return { ...ui, calls, handlers, profiles, folders, source, target, noteContext, center: ui.run('organizationCenter'),
    async settle() { for (let i = 0; i < 8; i++) await Promise.resolve(); }
  };
}

test('saving a folder captures filters, preserves exclusive date boundaries and opens a dynamic view', async () => {
  const w = workspace();
  w.get('q').value = 'modifiee:7j -annulée';
  w.get('dfrom').value = '2026-12-31'; w.get('dto').value = '2026-12-31';
  await w.get('saveSearchBtn').onclick();
  assert.equal(w.get('folderQuery').value, 'modifiee:7j -annulée');
  assert.equal(w.get('folderTo').value, '2026-12-31');
  w.get('folderName').value = 'Mes notes récentes';
  await w.get('saveFolderBtn').onclick();
  const request = w.calls.find(([cmd]) => cmd === 'save_smart_folder')[1];
  assert.equal(request.params.date_to, '2027-01-01T00:00:00.000Z');
  assert.equal(request.params.query, 'modifiee:7j -annulée');
  assert.equal(w.center.activeFolderId(), 'created-folder');
  assert(w.get('currentProfileTitle').textContent.includes('Mes notes récentes'));
  assert(w.get('folderModal').classList.contains('hidden'));
});

test('folder errors retain the form and stale responses cannot reopen a locked vault', async () => {
  const w = workspace();
  await w.center.openFolderEditor();
  w.get('folderName').value = 'À corriger'; w.get('folderQuery').value = 'apres:invalide';
  w.handlers.save_smart_folder = () => { throw new Error('Date invalide'); };
  await w.get('saveFolderBtn').onclick();
  assert.equal(w.get('folderName').value, 'À corriger');
  assert(w.get('folderStatus').textContent.includes('Date invalide'));
  assert(!w.get('folderModal').classList.contains('hidden'));
  let reply;
  w.handlers.run_smart_folder = () => new Promise(resolve => { reply = resolve; });
  w.get('cancelFolderBtn').onclick();
  const pending = w.center.openFolder('folder');
  await w.settle();
  w.context.concealSession();
  reply({ folder: w.folders[0], notes: [w.source] });
  await pending;
  assert.equal(w.center.activeFolderId(), null);
  assert.equal(w.get('foldersList').children.length, 0);
  assert.equal(w.get('notesGrid').children.length, 0);
});

test('editing a note refreshes its active folder instead of replacing it with a profile view', async () => {
  const w = workspace();
  await w.center.refreshFolders(); await w.center.openFolder('folder');
  await w.context.openEditor({ note_id: 'source' });
  w.get('noteContent').value = 'Ne correspond plus au dossier'; w.get('noteContent').oninput();
  w.handlers.run_smart_folder = () => ({ folder: w.folders[0], notes: [] });
  await w.get('saveNoteBtn').onclick();
  assert.equal(w.center.activeFolderId(), 'folder');
  assert.equal(w.run('currentNotes.length'), 0);
  assert(w.get('currentProfileTitle').textContent.includes('0 note'));
});

test('folder palette scope opens the selected saved search', async () => {
  const w = workspace();
  await w.key('k', { ctrlKey: true });
  w.get('commandQuery').value = '/Focus'; w.get('commandQuery').oninput(); await w.tick(150);
  assert.equal(w.get('commandResults').children.length, 1);
  await w.key('Enter');
  assert.equal(w.center.activeFolderId(), 'folder');
  assert.equal(w.get('q').value, 'tag:focus');
});

test('the context panel renders incoming, outgoing and missing links without loading attachments', async () => {
  const w = workspace();
  await w.context.viewNote(w.source); await w.settle();
  assert.equal(w.get('incomingLinks').children.length, 1);
  assert.equal(w.get('outgoingLinks').children.length, 2);
  assert(w.get('incomingTitle').textContent.includes('(1)'));
  assert(w.get('outgoingLinks').textContent.includes('Référence sans cible'));
  assert(!w.calls.some(([cmd]) => cmd === 'get_note' || cmd === 'search_notes'));
  await w.get('incomingLinks').children[0].children[0].onclick();
  assert(w.calls.some(([cmd, args]) => cmd === 'get_note' && args.id === 'target'));
});

test('protected notes do not request a link index and old contexts cannot overwrite a new reader', async () => {
  const w = workspace();
  w.context.fixture = { ...w.source, is_locked: true };
  w.run('viewingNote = fixture;');
  await w.center.loadReader(w.context.fixture);
  assert(w.get('contextStatus').textContent.includes('protégées'));
  assert(!w.calls.some(([cmd]) => cmd === 'get_note_context'));
  let reply;
  w.handlers.get_note_context = ({ id }) => id === 'source' ? new Promise(resolve => { reply = resolve; }) : { incoming: [], outgoing: [], incoming_total: 0, outgoing_total: 0 };
  await w.context.viewNote(w.source);
  await w.context.viewNote(w.target); await w.settle();
  reply(w.noteContext); await w.settle();
  assert.equal(w.get('incomingLinks').children.length, 0);
  assert.equal(w.get('outgoingLinks').children.length, 0);
});

test('missing references can create a target and ambiguous references require a choice', async () => {
  const w = workspace();
  await w.context.viewNote(w.source); await w.settle();
  await w.center.openReference('Absent');
  const row = w.get('referenceTargets').children[0];
  await row.children.find(n => n.textContent === 'Créer cette note').onclick();
  assert(w.calls.some(([cmd, args]) => cmd === 'create_note_from_reference' && args.sourceId === 'source' && args.reference === 'Absent'));
  assert(w.calls.some(([cmd, args]) => cmd === 'open_note_editor' && args.params.note_id === 'made'));
  await w.get('cancelNoteBtn').onclick();
  await w.context.viewNote(w.source); await w.settle();
  w.handlers.resolve_note_reference = () => ({ reference: 'Cible', label: 'Cible', status: 'ambiguous', can_create: false, targets: [{ id: 'a', title: 'Cible A', preview: '', profile_name: 'A' }, { id: 'b', title: 'Cible B', preview: '', profile_name: 'B' }] });
  const before = w.calls.filter(([cmd]) => cmd === 'get_note').length;
  await w.center.openReference('Cible');
  assert.equal(w.calls.filter(([cmd]) => cmd === 'get_note').length, before);
  assert(w.get('referenceTargets').textContent.includes('Cible A'));
  assert(w.get('referenceTargets').textContent.includes('Cible B'));
});

test('the editor picker inserts a stable ID link and the reader displays its alias', async () => {
  const w = workspace();
  await w.context.openEditor({ note_id: 'source' });
  const field = w.get('noteContent'); field.value = 'Avant  après'; field.selectionStart = field.selectionEnd = 6;
  await w.get('pickNoteLinkBtn').onclick();
  assert.equal(w.get('commandResults').children.length, 1);
  await w.get('commandResults').children[0].onclick();
  assert.equal(field.value, 'Avant [[target|Cible]] après');
  assert.equal(w.run('editorSession.token'), 'editor');
  const html = w.context.renderHighlighted('Avant [[target|Cible]] après', []);
  assert(html.includes('>[[Cible]]</span>'));
  assert(html.includes('data-ref="target"'));
  assert(!html.includes('>[[target|Cible]]'));
});

test('stable links do not shift highlight selection offsets, including Unicode and nested marks', () => {
  const w = workspace();
  const text = value => ({ nodeType: 3, textContent: value, childNodes: [], contains(n) { return n === this; } });
  const element = (children, dataset = {}) => ({ nodeType: 1, dataset, childNodes: children, get textContent() { return children.map(n => n.textContent).join(''); }, contains(n) { return n === this || children.some(c => c.contains(n)); } });
  const raw = '[[target-id|Cible]]', start = 2, end = start + [...raw].length;
  const alias = text('[[Cible]]'), after = text(' suite 🌍 fin');
  const link = element([alias], { sourceStart: String(start), sourceEnd: String(end) });
  const container = element([text('🔐 '), element([link]), after]);
  let range = { startContainer: after, startOffset: 1, endContainer: after, endOffset: 6 };
  w.context.window.getSelection = () => ({ rangeCount: 1, isCollapsed: false, getRangeAt: () => range });
  let offsets = w.context.getSelectionOffsets(container);
  assert.equal(offsets.start, end + 1); assert.equal(offsets.end, end + 6);
  range = { startContainer: alias, startOffset: 3, endContainer: alias, endOffset: 6 };
  offsets = w.context.getSelectionOffsets(container);
  assert.equal(offsets.start, start); assert.equal(offsets.end, end);
  const position = after.textContent.indexOf('fin');
  range = { startContainer: after, startOffset: position, endContainer: after, endOffset: position + 3 };
  offsets = w.context.getSelectionOffsets(container);
  assert.equal(offsets.start, end + [...after.textContent.slice(0, position)].length);
});

test('stable link copying uses a selected read-only field and clears it on dialog close', async () => {
  const w = workspace();
  await w.context.viewNote(w.source); await w.settle();
  const dialog = w.get('stableLinkBtn').onclick();
  await w.tick(60);
  assert.equal(w.get('sysInput').value, '[[source|Source]]');
  assert.equal(w.get('sysInput').readOnly, true);
  assert.equal(w.get('sysInput').selectionStart, 0);
  w.get('sysOk').onclick(); await dialog;
  assert.equal(w.get('sysInput').value, ''); assert.equal(w.get('sysInput').readOnly, false);
});

test('folder names and link previews remain text when they contain HTML', async () => {
  const w = workspace();
  const payload = '<img src=x onerror="bad()">';
  w.folders[0].name = payload;
  w.noteContext.incoming[0].title = payload;
  w.noteContext.incoming[0].preview = payload;
  await w.center.refreshFolders();
  const folderLabel = w.get('foldersList').children[0].children[0].children[0];
  assert.equal(folderLabel.textContent, payload); assert.equal(folderLabel.innerHTML, '');
  await w.context.viewNote(w.source); await w.settle();
  const preview = w.get('incomingLinks').children[0].children[0];
  assert(preview.textContent.includes(payload)); assert.equal(preview.innerHTML, '');
});

test('late organization dialogs cannot steal focus from a command palette opened afterwards', async () => {
  const w = workspace();
  await w.context.viewNote(w.source); await w.settle();
  let referenceReply;
  w.handlers.resolve_note_reference = () => new Promise(resolve => { referenceReply = resolve; });
  const reference = w.center.openReference('Absent');
  await w.run('commandCenter').openPalette();
  referenceReply({ reference: 'Absent', label: 'Absent', status: 'missing', targets: [], can_create: true });
  await reference;
  assert(w.get('referenceModal').classList.contains('hidden'));
  assert.equal(w.document.activeElement, w.get('commandQuery'));
  w.run('commandCenter').closePalette();
  let profilesReply;
  w.handlers.list_profiles = () => new Promise(resolve => { profilesReply = resolve; });
  const folder = w.center.openFolderEditor(); await w.settle();
  await w.run('commandCenter').openPalette();
  profilesReply(w.profiles); await folder;
  assert(w.get('folderModal').classList.contains('hidden'));
  assert.equal(w.document.activeElement, w.get('commandQuery'));
});

test('a folder save acknowledged after vault lock does not restore its name or open its view', async () => {
  const w = workspace();
  await w.center.openFolderEditor();
  w.get('folderName').value = 'Nom privé';
  let reply;
  w.handlers.save_smart_folder = () => new Promise(resolve => { reply = resolve; });
  const pending = w.get('saveFolderBtn').onclick();
  w.context.concealSession();
  reply({ id: 'late', name: 'Nom privé', params: params() });
  await pending;
  assert.equal(w.get('folderName').value, '');
  assert(w.get('folderModal').classList.contains('hidden'));
  assert.equal(w.get('foldersList').children.length, 0);
  assert(!w.calls.some(([cmd]) => cmd === 'run_smart_folder'));
});

test('a graph entity filter leaves the saved-folder context before showing its own results', async () => {
  const w = workspace();
  await w.center.refreshFolders(); await w.center.openFolder('folder');
  w.run('gDrag = { kind: "person", id: "person:yacine", title: "@yacine" }; gMoved = false;');
  await w.listeners.find(listener => listener.event === 'mouseup').callback();
  assert.equal(w.center.activeFolderId(), null);
  assert.equal(w.get('q').value, '@yacine');
});
