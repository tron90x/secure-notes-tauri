const { test: nodeTest } = require('node:test');
const test = (name, fn) => nodeTest(name, { timeout: 3000 }, fn);
const assert = require('node:assert/strict');
const { frontend, editorView } = require('./helpers/frontend.cjs');

function browserFile(name, contents, relative = '') {
  const bytes = Buffer.from(contents);
  return { name, webkitRelativePath: relative, size: bytes.length,
    async arrayBuffer() { return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length); } };
}

function workspace() {
  const ui = frontend(), calls = [], handlers = {}, downloads = [], blobs = new Map(), storage = [];
  const profiles = [{ id: 'p', name: 'Personnel' }];
  const notes = [{ id: 'free', title: '<img src=x onerror="bad()">', profile_id: 'p', profile_name: 'Personnel', is_locked: false },
    { id: 'locked', title: 'Titre privé à masquer', profile_id: 'p', profile_name: 'Personnel', is_locked: true }];
  const preview = { token: 'plan', notes_count: 1, protected_count: 0, attachments_count: 1, duplicates: 1, profiles_count: 1,
    ignored_files: 0, resource_bundles: 0, edited_files: 0, kind: 'markdown', notes: [notes[0]] };
  ui.context.fixtureProfiles = profiles;
  ui.run('profiles = fixtureProfiles; currentProfile = profiles[0];');
  ui.get('mainView').classList.remove('hidden');
  ui.context.localStorage.setItem = (...args) => storage.push(args);
  ui.context.URL = { createObjectURL(blob) { const url = `blob:export-${blobs.size}`; blobs.set(url, blob); return url; }, revokeObjectURL(url) { blobs.delete(url); } };
  const create = ui.document.createElement;
  ui.document.createElement = tag => {
    const node = create(tag);
    if (tag === 'a') node.click = () => downloads.push({ name: node.download, blob: blobs.get(node.href) });
    return node;
  };
  ui.context.invoke = async (cmd, args = {}) => {
    calls.push([cmd, args]);
    if (handlers[cmd]) return handlers[cmd](args);
    if (cmd === 'list_profiles') return profiles;
    if (cmd === 'list_transfer_notes') return notes;
    if (cmd === 'get_auto_lock_timeout') return 300;
    if (cmd === 'open_note_editor') return editorView();
    if (cmd === 'prepare_note_import') return preview;
    if (cmd === 'commit_note_import') return { added: 1, skipped: 0, profile_id: 'p' };
    if (cmd === 'export_notes') return { name: 'archive.snotetransfer', mime: 'application/x-securenotes-transfer', data: 'AAH/', notes_count: args.ids.length };
    return [];
  };
  return { ...ui, calls, handlers, downloads, blobs, storage, notes, preview, center: ui.run('transferCenter'),
    pick(files) { ui.get('transferFiles').onchange({ target: { files } }); },
    async settle() { for (let i = 0; i < 10; i++) await Promise.resolve(); }
  };
}

test('encrypted export validates passwords, masks protected titles and downloads the exact opaque bytes', async () => {
  const w = workspace();
  await w.center.open('export');
  assert.equal(w.get('transferFormat').value, 'encrypted');
  assert(!w.get('transferNotes').textContent.includes('Titre privé à masquer'));
  assert(w.get('transferNotes').textContent.includes('Note verrouillée'));
  w.get('transferPassword').value = w.get('transferConfirm').value = 'court';
  await w.get('exportNotesBtn').onclick();
  assert(!w.calls.some(([cmd]) => cmd === 'export_notes'));
  w.get('transferPassword').value = 'archive-password'; w.get('transferConfirm').value = 'différent';
  await w.get('exportNotesBtn').onclick();
  assert(w.get('transferStatus').textContent.includes('correspondent'));
  w.get('transferConfirm').value = 'archive-password';
  await w.get('exportNotesBtn').onclick();
  const request = w.calls.find(([cmd]) => cmd === 'export_notes')[1];
  assert.equal(request.password, 'archive-password'); assert.equal(request.ids.length, 2);
  assert.equal(w.get('transferPassword').value, ''); assert.equal(w.get('transferConfirm').value, '');
  assert.deepEqual([...new Uint8Array(await w.downloads[0].blob.arrayBuffer())], [0, 1, 255]);
  assert.deepEqual(w.storage, []);
});

test('plain Markdown and text formats deselect protected notes explicitly and keep labels as text', async () => {
  const w = workspace(); await w.center.open();
  w.get('transferFormat').value = 'markdown'; w.get('transferFormat').onchange();
  assert(w.get('transferStatus').textContent.includes('1 note'));
  assert.equal(w.get('transferNotes').children[1].children[0].disabled, true);
  assert.equal(w.get('transferNotes').children[1].children[0].checked, false);
  const name = w.get('transferNotes').children[0].children[1].children[0];
  assert.equal(name.textContent, w.notes[0].title); assert.equal(name.innerHTML, '');
  await w.get('exportNotesBtn').onclick();
  let request = w.calls.findLast(([cmd]) => cmd === 'export_notes')[1];
  assert.deepEqual([...request.ids], ['free']); assert.equal(request.password, null);
  w.get('transferFormat').value = 'text'; w.get('transferFormat').onchange();
  await w.get('exportNotesBtn').onclick();
  request = w.calls.findLast(([cmd]) => cmd === 'export_notes')[1];
  assert.equal(request.format, 'text'); assert.deepEqual([...request.ids], ['free']);
});

test('import preparation retains relative paths and confirms destination/copy policy only on commit', async () => {
  const w = workspace(); await w.center.open('import');
  w.pick([browserFile('note.md', '# Note 🌍', 'Root/note.md'), browserFile('pic.bin', [0, 255], 'Root/pic.bin')]);
  await w.get('prepareImportBtn').onclick();
  const prepared = w.calls.find(([cmd]) => cmd === 'prepare_note_import')[1];
  assert.equal(prepared.files[0].name, 'Root/note.md');
  assert.equal(Buffer.from(prepared.files[0].data, 'base64').toString(), '# Note 🌍');
  assert(!w.calls.some(([cmd]) => cmd === 'commit_note_import'));
  assert(w.get('transferPreviewSummary').textContent.includes('1 identifiant'));
  assert.equal(w.get('transferPreview').children[0].innerHTML, '');
  w.get('transferPreserveProfiles').checked = false; w.get('transferPreserveProfiles').onchange();
  w.get('transferDestination').value = 'p'; w.get('transferCopies').checked = true;
  await w.get('commitImportBtn').onclick();
  const committed = w.calls.find(([cmd]) => cmd === 'commit_note_import')[1];
  assert.equal(committed.destinationProfileId, 'p'); assert.equal(committed.preserveProfiles, false); assert.equal(committed.copies, true);
  assert(w.get('transferStatus').textContent.includes('1 note(s) ajoutée'));
  assert.equal(w.get('commitImportBtn').disabled, true);
});

test('protected imports require an archive password and disable copies after preparation', async () => {
  const w = workspace(); await w.center.open('import');
  w.preview.protected_count = 1; w.preview.notes = [w.notes[1]];
  w.pick([browserFile('archive.snotetransfer', '{}')]);
  await w.get('prepareImportBtn').onclick();
  assert(!w.calls.some(([cmd]) => cmd === 'prepare_note_import'));
  w.get('transferImportPassword').value = 'archive-password';
  await w.get('prepareImportBtn').onclick();
  assert.equal(w.get('transferImportPassword').value, '');
  assert.equal(w.get('transferCopies').disabled, true); assert.equal(w.get('transferCopies').checked, false);
  assert(!w.get('transferPreview').textContent.includes('Titre privé à masquer'));
});

test('failed import commits preserve the prepared token and allow a retry', async () => {
  const w = workspace(); await w.center.open('import'); w.pick([browserFile('note.txt', 'text')]);
  await w.get('prepareImportBtn').onclick();
  let fail = true;
  w.handlers.commit_note_import = () => { if (fail) throw new Error('Disque plein'); return { added: 1, skipped: 0, profile_id: 'p' }; };
  await w.get('commitImportBtn').onclick();
  assert(w.get('transferStatus').textContent.includes('Disque plein'));
  assert.equal(w.get('commitImportBtn').disabled, false);
  fail = false; await w.get('commitImportBtn').onclick();
  assert.equal(w.calls.filter(([cmd, args]) => cmd === 'commit_note_import' && args.token === 'plan').length, 2);
});

test('closing a prepared import discards the server plan and clears filenames and passwords', async () => {
  const w = workspace(); await w.center.open('import'); w.pick([browserFile('private.md', 'secret')]);
  await w.get('prepareImportBtn').onclick(); w.get('transferImportPassword').value = 'not retained';
  w.get('closeTransferBtn').onclick();
  assert(w.calls.some(([cmd, args]) => cmd === 'discard_note_import' && args.token === 'plan'));
  assert.equal(w.get('transferFileNames').textContent, ''); assert.equal(w.get('transferImportPassword').value, '');
  assert(w.get('transferModal').classList.contains('hidden'));
});

test('late export and import responses cannot download files or restore previews after vault lock', async () => {
  const w = workspace(); await w.center.open();
  w.get('transferPassword').value = w.get('transferConfirm').value = 'archive-password';
  let exportReply;
  w.handlers.export_notes = () => new Promise(resolve => { exportReply = resolve; });
  const exporting = w.get('exportNotesBtn').onclick();
  w.context.concealSession();
  exportReply({ name: 'late.zip', data: 'AA==', mime: 'application/zip', notes_count: 1 }); await exporting;
  assert.equal(w.downloads.length, 0);
  w.get('mainView').classList.remove('hidden');
  await w.center.open('import'); w.pick([browserFile('private.txt', 'secret')]);
  let importReply;
  w.handlers.prepare_note_import = () => new Promise(resolve => { importReply = resolve; });
  const preparing = w.get('prepareImportBtn').onclick(); await w.settle();
  w.context.concealSession(); importReply({ ...w.preview, token: 'late' }); await preparing;
  assert.equal(w.get('transferPreview').children.length, 0); assert.equal(w.get('transferStatus').textContent, '');
  assert(w.calls.some(([cmd, args]) => cmd === 'discard_note_import' && args.token === 'late'));
});

test('file reads that finish after locking are not sent to the backend', async () => {
  const w = workspace(); await w.center.open('import');
  let read;
  const file = browserFile('secret.txt', 'secret'); file.arrayBuffer = () => new Promise(resolve => { read = resolve; });
  w.pick([file]); const preparing = w.get('prepareImportBtn').onclick();
  w.context.concealSession(); read(new Uint8Array([1, 2]).buffer); await preparing;
  assert(!w.calls.some(([cmd]) => cmd === 'prepare_note_import'));
});

test('palette export preserves the selected reader note before closing the reader', async () => {
  const w = workspace();
  await w.context.viewNote({ id: 'locked', profile_id: 'p', title: 'Selected', content: 'body', is_locked: false, modified_history: [], tags: [], versions: [], created_at: '2026-10-25T12:00:00Z' });
  await w.key('k', { ctrlKey: true });
  w.get('commandQuery').value = '>export'; w.get('commandQuery').oninput(); await w.tick(150);
  await w.key('Enter');
  assert.equal(w.get('transferNotes').children[0].children[0].checked, false);
  assert.equal(w.get('transferNotes').children[1].children[0].checked, true);
});

test('oversized file selections are rejected before reading their bytes', async () => {
  const w = workspace(); await w.center.open('import');
  let read = false;
  w.pick([{ name: 'huge.zip', size: 97 * 1024 * 1024, async arrayBuffer() { read = true; return new ArrayBuffer(0); } }]);
  assert.equal(w.get('prepareImportBtn').disabled, true);
  assert(w.get('transferStatus').textContent.includes('volumineux'));
  assert.equal(read, false);
});

test('native window close is deferred while an export is still being prepared', async () => {
  const w = workspace(); let onClose;
  w.context.window.__TAURI__ = { window: { getCurrentWindow: () => ({ async onCloseRequested(callback) { onClose = callback; } }) } };
  await w.context.enterMain(); await w.center.open();
  w.get('transferPassword').value = w.get('transferConfirm').value = 'archive-password';
  let reply;
  w.handlers.export_notes = () => new Promise(resolve => { reply = resolve; });
  const pending = w.get('exportNotesBtn').onclick();
  let prevented = false; await onClose({ preventDefault() { prevented = true; } });
  assert.equal(prevented, true);
  reply({ name: 'ready.snotetransfer', mime: 'application/octet-stream', data: 'AA==', notes_count: 1 }); await pending;
  assert.equal(w.center.isBusy(), false);
});

test('a slow transfer opening does not take focus away from a quick capture opened above it', async () => {
  const w = workspace();
  let reply;
  w.handlers.list_transfer_notes = () => new Promise(resolve => { reply = resolve; });
  const opening = w.center.open(); await w.settle();
  await w.run('commandCenter').openCapture();
  assert.equal(w.document.activeElement, w.get('captureContent'));
  reply(w.notes); await opening;
  assert.equal(w.document.activeElement, w.get('captureContent'));
});

test('the native directory picker prepares an import without browser directory APIs', async () => {
  const w = workspace(); await w.center.open('import');
  w.handlers.choose_directory_import = () => ({ ...w.preview, source: 'Mes notes' });
  await w.get('pickImportFolderBtn').onclick();
  assert.equal(w.get('transferFileNames').textContent, 'Mes notes');
  assert.equal(w.get('commitImportBtn').disabled, false);
  assert(!w.calls.some(([cmd]) => cmd === 'prepare_note_import'));
  await w.get('commitImportBtn').onclick();
  assert(w.calls.some(([cmd, args]) => cmd === 'commit_note_import' && args.token === 'plan'));
});

test('native picker cancellation retains a prepared file import and stale folder results are discarded', async () => {
  const w = workspace(); await w.center.open('import'); w.pick([browserFile('note.txt', 'text')]);
  await w.get('prepareImportBtn').onclick();
  w.handlers.choose_directory_import = () => null;
  await w.get('pickImportFolderBtn').onclick();
  assert.equal(w.get('commitImportBtn').disabled, false);
  let reply;
  w.handlers.choose_directory_import = () => new Promise(resolve => { reply = resolve; });
  const choosing = w.get('pickImportFolderBtn').onclick();
  w.context.concealSession(); reply({ ...w.preview, token: 'native-late' }); await choosing;
  assert(w.get('transferModal').classList.contains('hidden'));
  assert(w.calls.some(([cmd, args]) => cmd === 'discard_note_import' && args.token === 'native-late'));
});
