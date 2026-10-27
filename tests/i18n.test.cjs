const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { frontend } = require('./helpers/frontend.cjs');

test('Kabyle covers every French key and preserves interpolation parameters and search operators', () => {
  const { run } = frontend();
  const strings = JSON.parse(run('JSON.stringify(STRINGS)'));
  assert.deepEqual(Object.keys(strings.kab).sort(), Object.keys(strings.fr).sort());
  const tokens = text => [...text.matchAll(/\{\w+\}/g)].map(m => m[0]).sort();
  for (const [key, french] of Object.entries(strings.fr)) {
    const kabyle = strings.kab[key];
    assert.equal(Array.isArray(kabyle), Array.isArray(french), key);
    const values = Array.isArray(kabyle) ? kabyle : [kabyle];
    if (Array.isArray(french)) assert.equal(kabyle.length, french.length, key);
    values.forEach((text, i) => {
      if (key !== 'strength_levels' || i) assert(text.trim(), key);
      assert(!/i usentem|weggz|d'inactivité|récente/.test(text), key);
      assert.deepEqual(tokens(text), tokens(Array.isArray(french) ? french[i] : french), key);
    });
  }
  for (const operator of ['modifiee:7j', 'avec:piece-jointe', 'sans:tag', 'statut:libre', 'statut:verrouillee']) {
    assert(strings.kab.search_help.includes(operator), operator);
  }
  const html = fs.readFileSync(path.join(__dirname, '../frontend/index.html'), 'utf8');
  const scripts = ['app.js', 'commands.js', 'organization.js', 'transfer.js'].map(name => fs.readFileSync(path.join(__dirname, '../frontend', name), 'utf8')).join('\n');
  for (const [, key] of html.matchAll(/data-i18n(?:-ph|-title|-aria)?="([^"]+)"/g)) assert(Object.hasOwn(strings.kab, key), key);
  for (const [, key] of scripts.matchAll(/\b(?:t|tr)\("([^"]+)"/g)) assert(Object.hasOwn(strings.kab, key), key);
});

test('language selection updates document language, placeholders, tooltips and accessible labels', () => {
  const { context, document, get, run } = frontend();
  document.documentElement = {};
  const text = get('translated-text'), placeholder = get('translated-placeholder'), title = get('translated-title'), aria = get('translated-aria');
  text.dataset.i18n = 'empty_trash_btn'; placeholder.dataset.i18nPh = 'goal_ph'; title.dataset.i18nTitle = 'color_yellow'; aria.dataset.i18nAria = 'protection_label';
  document.querySelectorAll = selector => ({ '[data-i18n]': [text], '[data-i18n-ph]': [placeholder], '[data-i18n-title]': [title], '[data-i18n-aria]': [aria] }[selector] || []);
  context.setLang('kab');
  assert.equal(document.documentElement.lang, 'kab');
  assert.equal(document.title, run('t("app_title")'));
  assert.equal(text.innerHTML, 'Silem taqecwalt');
  assert.equal(placeholder.placeholder, run('t("goal_ph")'));
  assert.equal(title.title, 'Awraɣ');
  assert.equal(aria.getAttribute('aria-label'), 'Ammesten n tezmilt');
  context.setLang('fr');
  assert.equal(document.documentElement.lang, 'fr');
  assert.equal(text.innerHTML, 'Vider la corbeille');
  assert.equal(placeholder.placeholder, 'Objectif en mots/jour');
});

test('Kabyle dynamic note rendering and confirmations preserve user text and hide protected content', async () => {
  const { context, get, run } = frontend();
  context.setLang('kab');
  context.renderNotes([{ id: 'n', title: '<b>{count}</b>', content: 'Agbur', is_locked: false, created_at: '2026-10-27T10:00:00Z', modified_history: ['2026-10-27T11:00:00Z'] },
    { id: 'locked', title: 'SECRET', content: 'SECRET', is_locked: true, created_at: '2026-10-27T10:00:00Z', modified_history: [] }]);
  const cards = get('notesGrid').children.map(node => node.innerHTML).join('');
  assert(cards.includes('&lt;b&gt;{count}&lt;/b&gt;'));
  assert(cards.includes('1 n yibeddilen'));
  assert(!cards.includes('SECRET'));
  assert(!/Note verrouillée|modification/.test(cards));
  assert.equal(run('tr("open_named", { name: "{count} <b>" })'), 'Ldi « {count} <b> »');
  const confirmation = run('sysConfirm(t("destroy_confirm"))');
  assert.equal(get('sysTitle').textContent, '❓ Sentem');
  assert.equal(get('sysOk').textContent, 'Sentem');
  assert(get('sysMsg').textContent.includes('i lebda'));
  get('sysCancel').onclick();
  assert.equal((await confirmation).ok, false);
});

test('backend errors are translated at the bridge while session detection and technical details stay intact', async () => {
  const { context, run } = frontend();
  const app = fs.readFileSync(path.join(__dirname, '../frontend/app.js'), 'utf8');
  run(app.slice(0, app.indexOf('const $ =')));
  context.setLang('kab');
  context.window.__TAURI__ = { core: { invoke: async () => { throw 'Mot de passe incorrect ou fichier corrompu'; } } };
  await assert.rejects(context.invoke('unlock_vault'), error => error === 'Awal uffir d arameɣtu neɣ afaylu yexṣer');
  context.window.__TAURI__.core.invoke = async () => { throw 'Coffre verrouillé'; };
  await assert.rejects(context.invoke('get_note'), error => error === 'Coffre verrouillé');
  assert.equal(run('localizeError("Fichier de note absent : notes/français-{count}.md")'), 'Afaylu n tezmilt ulac-it: notes/français-{count}.md');
  assert.equal(run('localizeError("Permission denied (os error 13)")'), 'Permission denied (os error 13)');
  assert(run('localizeError("Brouillon conservé — enregistrement de la note impossible : Profil supprimé")').includes('Amaɣnu yettwakkes'));
  context.setLang('fr');
  assert.equal(run('localizeError("Mot de passe incorrect ou fichier corrompu")'), 'Mot de passe incorrect ou fichier corrompu');
});
