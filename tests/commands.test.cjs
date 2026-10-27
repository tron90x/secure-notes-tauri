const { test: nodeTest } = require('node:test');
const test = (name, fn) => nodeTest(name, { timeout: 3000 }, fn);
const assert = require('node:assert/strict');
const { frontend, editorView } = require('./helpers/frontend.cjs');

function workspace(options = {}) {
  const ui = frontend();
  const profiles = options.profiles ?? [{ id: 'p', name: 'Personnel' }, { id: 'work', name: 'Travail' }];
  const calls = [], handlers = {}, writes = [];
  let serial = 0;
  ui.context.fixtureProfiles = profiles;
  ui.run('profiles = fixtureProfiles; currentProfile = profiles[0] || null;');
  ui.get('mainView').classList.remove('hidden');
  ui.context.localStorage.setItem = (...args) => writes.push(args);
  const metadata = { notes: [{ id: 'n', title: 'Projet budget', profile_id: 'p', profile_name: 'Personnel', is_locked: false }], profiles, tags: [['travail', 2]] };
  ui.context.invoke = async (cmd, args = {}) => {
    calls.push([cmd, args]);
    if (handlers[cmd]) return handlers[cmd](args);
    if (cmd === 'search_command_items') return metadata;
    if (cmd === 'list_profiles') return profiles;
    if (cmd === 'get_auto_lock_timeout') return 300;
    if (cmd === 'get_note') return { id: args.id, profile_id: 'p', title: 'Projet budget', content: 'Texte complet', modified_history: [], versions: [], created_at: '2026-10-25T12:00:00Z', tags: [] };
    if (cmd === 'open_note_editor') {
      const id = ++serial;
      return editorView({ token: `token-${id}`, draft_id: `draft-${id}`, profile_id: args.params.profile_id || 'p', note_id: args.params.note_id ?? null });
    }
    if (cmd === 'save_note_editor') return { note_id: args.params.commit ? 'created' : null, protected: false, committed: args.params.commit };
    if (cmd === 'create_profile') { const p = { id: `profile-${profiles.length}`, name: args.name }; profiles.push(p); return p; }
    return [];
  };
  return { ...ui, profiles, calls, handlers, writes, metadata, center: ui.run('commandCenter'),
    async query(value) { ui.get('commandQuery').value = value; ui.get('commandQuery').oninput(); await ui.tick(150); },
    rows() { return ui.get('commandResults').children; },
    saves() { return calls.filter(([cmd]) => cmd === 'save_note_editor').map(([, args]) => args.params); }
  };
}

test('locked shortcuts request unlock without accepting plaintext and resume the capture afterwards', async () => {
  const w = workspace();
  w.get('mainView').classList.add('hidden');
  await w.key('N', { ctrlKey: true, shiftKey: true });
  assert(w.get('quickCaptureModal').classList.contains('hidden'));
  assert(w.get('vaultMsg').textContent.includes('Déverrouillez'));
  assert.equal(w.document.activeElement, w.get('unlockPass'));
  assert(!w.calls.some(([cmd]) => cmd === 'open_note_editor' || cmd === 'search_command_items'));
  w.get('mainView').classList.remove('hidden');
  await w.center.onUnlocked();
  assert(!w.get('quickCaptureModal').classList.contains('hidden'));
  assert.equal(w.document.activeElement, w.get('captureContent'));
  assert.equal(w.get('captureContent').value, '');
  assert.deepEqual(w.writes, []);
});

test('palette supports command-only scope, cyclic keyboard selection and focus restoration', async () => {
  const w = workspace();
  w.get('q').focus();
  await w.key('k', { ctrlKey: true });
  assert.equal(w.document.activeElement, w.get('commandQuery'));
  const reads = w.calls.filter(([cmd]) => cmd === 'search_command_items').length;
  await w.query('>');
  assert.equal(w.calls.filter(([cmd]) => cmd === 'search_command_items').length, reads);
  assert.equal(w.rows().length, 14);
  await w.key('ArrowUp');
  assert.equal(w.get('commandQuery').getAttribute('aria-activedescendant'), 'command-result-13');
  await w.key('ArrowDown');
  assert.equal(w.rows()[0].getAttribute('aria-selected'), 'true');
  await w.key('Escape');
  assert(w.get('commandModal').classList.contains('hidden'));
  assert.equal(w.document.activeElement, w.get('q'));
  assert.equal(w.rows().length, 0);
});

test('stale metadata replies cannot replace newer search results or repopulate a locked palette', async () => {
  const w = workspace();
  await w.center.openPalette();
  let oldReply;
  w.handlers.search_command_items = ({ query }) => query === 'ancien' ? new Promise(resolve => { oldReply = resolve; }) : { notes: [{ id: 'new', title: 'Nouveau résultat', profile_name: 'Personnel' }], profiles: [], tags: [] };
  w.get('commandQuery').value = 'ancien';
  w.get('commandQuery').oninput();
  const old = w.tick(150);
  await w.query('nouveau');
  oldReply({ notes: [{ id: 'old', title: 'Ancien résultat', profile_name: 'Personnel' }], profiles: [], tags: [] });
  await old;
  assert(w.rows()[0].textContent.includes('Nouveau résultat'));
  assert(!w.rows().some(row => row.textContent.includes('Ancien résultat')));
  let lateReply;
  w.handlers.search_command_items = () => new Promise(resolve => { lateReply = resolve; });
  w.get('commandQuery').oninput();
  const late = w.tick(150);
  w.context.concealSession();
  lateReply({ notes: [{ id: 'late', title: 'Secret tardif', profile_name: 'Personnel' }], profiles: [], tags: [] });
  await late;
  assert.equal(w.rows().length, 0);
  assert.equal(w.get('commandQuery').value, '');
  assert(w.get('commandModal').classList.contains('hidden'));
});

test('palette renders metadata as text and masks locked titles even in unexpected backend responses', async () => {
  const w = workspace();
  const injected = '<img src=x onerror="bad()">';
  w.metadata.notes = [{ id: 'n', title: injected, profile_name: injected }, { id: 'secret', title: 'Titre confidentiel', profile_name: 'Personnel', is_locked: true }];
  await w.center.openPalette();
  const labels = w.rows().map(row => row.textContent);
  assert(labels.some(label => label.includes(injected)));
  assert(!labels.some(label => label.includes('Titre confidentiel')));
  assert(labels.some(label => label.includes('Note verrouillée')));
  const row = w.rows().find(row => row.textContent.includes(injected));
  assert.equal(row.innerHTML, '');
  assert.equal(row.children[1].children[0].innerHTML, '');
});

test('opening a palette note loads only that note and preserves the previous search context', async () => {
  const w = workspace();
  w.get('q').value = 'filtre existant';
  await w.center.openPalette();
  await w.query('budget');
  await w.key('Enter');
  assert.equal(w.get('vContent').innerHTML, 'Texte complet');
  assert.equal(w.get('q').value, 'filtre existant');
  assert.equal(w.calls.filter(([cmd]) => cmd === 'get_note').length, 1);
  assert(!w.calls.some(([cmd]) => cmd === 'list_notes' || cmd === 'search_notes'));
});

test('profile and tag prefixes route to the selected profile or a vault-wide tag filter', async () => {
  const w = workspace();
  await w.center.openPalette();
  await w.query('@Travail');
  assert(w.rows().every(row => row.textContent.includes('Profil')));
  const work = w.rows().find(row => row.textContent.includes('Travail'));
  await work.onclick();
  assert.equal(w.run('currentProfile.id'), 'work');
  assert(w.calls.some(([cmd, args]) => cmd === 'list_notes' && args.profileId === 'work'));
  await w.center.openPalette();
  await w.query('#travail');
  await w.key('Enter');
  const search = w.calls.findLast(([cmd]) => cmd === 'search_notes')[1].params;
  assert.equal(search.profile_id, null);
  assert.equal(search.tag, 'travail');
});

test('quick capture saves drafts, applies profile changes and publishes with Ctrl+Enter', async () => {
  const w = workspace();
  await w.key('N', { ctrlKey: true, shiftKey: true });
  const firstLine = 'Une idée 🌍';
  w.get('captureContent').value = `${firstLine}\nSuite exacte`;
  w.get('captureContent').oninput();
  w.get('captureProfile').value = 'work';
  w.get('captureProfile').onchange();
  await w.tick(1200);
  assert.equal(w.saves()[0].commit, false);
  assert.equal(w.saves()[0].profile_id, 'work');
  assert.equal(w.saves()[0].content.title, firstLine);
  assert.equal(w.saves()[0].content.content, `${firstLine}\nSuite exacte`);
  assert(w.get('captureStatus').textContent.includes('Brouillon chiffré'));
  assert.equal(w.get('captureStatus').getAttribute('aria-label'), w.get('captureStatus').textContent);
  await w.key('Enter', { ctrlKey: true });
  assert.equal(w.saves().at(-1).commit, true);
  assert(w.get('quickCaptureModal').classList.contains('hidden'));
  assert.equal(w.get('captureContent').value, '');
  assert.equal(w.run('currentProfile.id'), 'work');
  assert.deepEqual(w.writes, []);
});

test('capture uses an independent editor token and returns to an ongoing full-editor draft', async () => {
  const w = workspace();
  await w.context.openEditor({ note_id: 'n', profile_id: 'p' });
  w.get('noteContent').value = 'Rédaction principale';
  w.get('noteContent').oninput();
  w.get('noteContent').focus();
  const mainToken = w.run('editorSession.token');
  await w.center.openCapture();
  assert.equal(w.run('editorSession.token'), mainToken);
  assert(!w.calls.some(([cmd]) => cmd === 'close_note_editor'));
  w.get('captureContent').value = 'Idée séparée';
  w.get('captureContent').oninput();
  await w.get('saveCaptureBtn').onclick();
  assert.equal(w.get('noteContent').value, 'Rédaction principale');
  assert.equal(w.run('editorSession.token'), mainToken);
  assert.equal(w.document.activeElement, w.get('noteContent'));
  assert.notEqual(w.saves().at(-1).token, mainToken);
});

test('capture without profiles creates one before accepting note content and does not publish empty notes', async () => {
  const w = workspace({ profiles: [] });
  await w.center.openCapture();
  assert.equal(w.get('captureContent').disabled, true);
  assert(!w.get('captureProfileSetup').classList.contains('hidden'));
  w.get('captureProfileName').value = 'Inbox';
  await w.get('createCaptureProfileBtn').onclick();
  assert.equal(w.get('captureContent').disabled, false);
  assert.equal(w.get('captureProfile').value, 'profile-0');
  await w.get('saveCaptureBtn').onclick();
  assert.equal(w.saves().length, 0);
  assert(w.get('captureStatus').textContent.includes('Saisissez'));
  await w.get('closeCaptureBtn').onclick();
  assert(w.get('quickCaptureModal').classList.contains('hidden'));
});

test('capture write failures retain the popup and its text until a successful retry', async () => {
  const w = workspace();
  let fail = true;
  w.handlers.save_note_editor = ({ params }) => {
    if (fail) throw new Error('Disque plein');
    return { note_id: params.commit ? 'created' : null, protected: false, committed: params.commit };
  };
  await w.center.openCapture();
  w.get('captureContent').value = 'À conserver';
  w.get('captureContent').oninput();
  await w.tick(1200);
  assert(w.get('captureStatus').textContent.includes('Disque plein'));
  assert.equal(await w.center.finishCapture(false), false);
  assert.equal(w.get('captureContent').value, 'À conserver');
  assert(!w.get('quickCaptureModal').classList.contains('hidden'));
  fail = false;
  await w.get('saveCaptureBtn').onclick();
  assert(w.get('quickCaptureModal').classList.contains('hidden'));
});

test('capture can transfer its durable draft into the full editor', async () => {
  const w = workspace();
  await w.center.openCapture();
  w.get('captureContent').value = 'À développer';
  w.get('captureContent').oninput();
  w.handlers.open_note_editor = ({ params }) => editorView({ token: 'full-token', profile_id: params.profile_id, content: { ...editorView().content, content: 'À développer' } });
  await w.get('captureFullBtn').onclick();
  const opened = w.calls.findLast(([cmd]) => cmd === 'open_note_editor')[1].params;
  assert.equal(opened.draft_id, 'draft-1');
  assert.equal(w.get('noteContent').value, 'À développer');
  assert(w.get('quickCaptureModal').classList.contains('hidden'));
  assert(!w.get('noteModal').classList.contains('hidden'));
});

test('vault locking queues both draft snapshots before dropping either editor session', async () => {
  const w = workspace();
  await w.context.openEditor({ profile_id: 'p' });
  w.get('noteContent').value = 'Brouillon principal';
  w.get('noteContent').oninput();
  await w.center.openCapture();
  w.get('captureContent').value = 'Brouillon rapide';
  w.get('captureContent').oninput();
  const completed = [];
  w.handlers.save_note_editor = () => new Promise(resolve => completed.push(resolve));
  const locking = w.context.lockAndReload();
  assert.equal(w.saves().length, 2);
  assert(w.saves().every(params => params.commit === false));
  assert(!w.calls.some(([cmd]) => cmd === 'close_note_editor' || cmd === 'lock_vault'));
  assert.equal(w.get('noteContent').value, '');
  assert.equal(w.get('captureContent').value, '');
  completed.forEach(resolve => resolve({ note_id: null, protected: false, committed: false }));
  await locking;
  assert.equal(w.calls.at(-1)[0], 'lock_vault');
});

test('palette focus trapping keeps Enter on its close button out of command execution', async () => {
  const w = workspace();
  await w.center.openPalette();
  w.get('commandModal').querySelectorAll = () => [w.get('closeCommandsBtn'), w.get('commandQuery'), ...w.rows()];
  const tab = await w.key('Tab');
  assert.equal(tab.defaultPrevented, true);
  assert.equal(w.document.activeElement, w.get('closeCommandsBtn'));
  const enter = await w.key('Enter');
  assert.equal(enter.defaultPrevented, false);
  assert(!w.calls.some(([cmd]) => cmd === 'open_note_editor'));
  await w.get('closeCommandsBtn').onclick();
  assert(w.get('commandModal').classList.contains('hidden'));
});

test('native close waits for both editors and leaves both intact if capture saving fails', async () => {
  const w = workspace();
  let onClose, destroyed = 0, fail = true;
  w.context.window.__TAURI__ = { window: { getCurrentWindow: () => ({
    async onCloseRequested(callback) { onClose = callback; }, async destroy() { destroyed++; }
  }) } };
  await w.context.enterMain();
  await w.context.openEditor({ profile_id: 'p' });
  w.get('noteContent').value = 'Texte principal';
  w.get('noteContent').oninput();
  await w.center.openCapture();
  w.get('captureContent').value = 'Idée rapide';
  w.get('captureContent').oninput();
  w.handlers.save_note_editor = ({ params }) => {
    if (fail) throw new Error('Disque indisponible');
    return { note_id: null, protected: false, committed: params.commit };
  };
  await onClose({ preventDefault() {} });
  assert.equal(destroyed, 0);
  assert.equal(w.get('noteContent').value, 'Texte principal');
  assert.equal(w.get('captureContent').value, 'Idée rapide');
  fail = false;
  await onClose({ preventDefault() {} });
  assert.equal(destroyed, 1);
  assert(w.saves().some(params => params.content.content === 'Texte principal'));
  assert(w.saves().some(params => params.content.content === 'Idée rapide'));
});

test('a late capture autosave cannot restore cleared plaintext after vault expiry', async () => {
  const w = workspace();
  await w.center.openCapture();
  let reply;
  w.handlers.save_note_editor = () => new Promise(resolve => { reply = resolve; });
  w.get('captureContent').value = 'Texte éphémère';
  w.get('captureContent').oninput();
  const pending = w.tick(1200);
  w.context.concealSession();
  reply({ note_id: null, protected: false, committed: false });
  await pending;
  assert(w.get('quickCaptureModal').classList.contains('hidden'));
  assert.equal(w.get('captureContent').value, '');
  assert.equal(w.get('captureStatus').textContent, '');
  assert.equal(w.get('captureStatus').getAttribute('aria-label'), '');
});

test('palette navigation preserves a full-editor draft whose new password is incomplete', async () => {
  const w = workspace();
  await w.context.openEditor({ profile_id: 'p' });
  w.get('noteContent').value = 'Travail non perdu';
  w.get('noteContent').oninput();
  w.get('noteProtection').value = 'set';
  w.get('notePass').value = 'abc';
  await w.center.openPalette();
  await w.query('@Travail');
  await w.rows().find(row => row.textContent.includes('Travail')).onclick();
  assert(!w.get('noteModal').classList.contains('hidden'));
  assert.equal(w.get('noteContent').value, 'Travail non perdu');
  assert.equal(w.run('currentProfile.id'), 'p');
  assert.equal(w.saves().length, 0);
});

test('Escape cancels password dialogs without dismissing the underlying screen', async () => {
  const w = workspace();
  w.get('draftsModal').classList.remove('hidden');
  const answer = w.context.sysDialog({ title: 'Mot de passe', msg: 'Saisie', input: true, password: true });
  w.get('sysInput').value = 'temporaire';
  let stopped = false;
  w.get('sysModal').onkeydown({ key: 'Escape', preventDefault() {}, stopPropagation() { stopped = true; } });
  assert.equal((await answer).ok, false);
  assert.equal(stopped, true);
  assert.equal(w.get('sysInput').value, '');
  assert(!w.get('draftsModal').classList.contains('hidden'));
});

test('a slow capture opening cannot steal focus from a palette opened on top', async () => {
  const w = workspace();
  let resolveProfiles;
  w.handlers.list_profiles = () => new Promise(resolve => { resolveProfiles = resolve; });
  const opening = w.center.openCapture();
  await w.center.openPalette();
  resolveProfiles(w.profiles);
  await opening;
  assert.equal(w.document.activeElement, w.get('commandQuery'));
  await w.key('Escape');
  assert(!w.get('quickCaptureModal').classList.contains('hidden'));
  assert.equal(w.document.activeElement, w.get('captureContent'));
});
