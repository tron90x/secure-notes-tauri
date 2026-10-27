const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function editorView(overrides = {}) {
  return { token: 'editor-token', draft_id: 'draft-id', profile_id: 'p', note_id: null, protected: false,
    content: { title: '', content: '', attachment_name: null, attachment_data: null, wallpaper: 'aurora', glass: true, tags: [] }, ...overrides };
}

function frontend() {
  const elements = new Map(), timers = new Map(), listeners = [];
  const document = {};
  let timerId = 0;
  function element() {
    const classes = new Set(), attributes = new Map();
    return { value: '', _text: '', _html: '', children: [], style: {}, dataset: {}, files: [], isConnected: true, tabIndex: 0,
      get textContent() { return this.children.length ? this.children.map(c => c.textContent).join('') : this._text; },
      set textContent(value) { this._text = value; this.children = []; this._html = ''; },
      get innerHTML() { return this._html; },
      set innerHTML(value) { this._html = value; this.children = []; this._text = ''; },
      classList: { add(c) { classes.add(c); }, remove(c) { classes.delete(c); }, toggle(c, on) { if (on ?? !classes.has(c)) classes.add(c); else classes.delete(c); }, contains(c) { return classes.has(c); } },
      appendChild(c) { this.children.push(c); c.parentElement = this; },
      replaceChildren(...children) { this.children = []; this._text = this._html = ''; children.forEach(c => this.appendChild(c)); },
      setAttribute(name, value) { attributes.set(name, String(value)); }, getAttribute(name) { return attributes.get(name) ?? null; }, removeAttribute(name) { attributes.delete(name); },
      addEventListener() {}, querySelectorAll() { return []; }, scrollIntoView() {},
      getClientRects() { for (let n = this; n; n = n.parentElement) if (n.classList?.contains('hidden')) return []; return [{}]; },
      focus() { document.activeElement = this; },
      select() { this.selectionStart = 0; this.selectionEnd = this.value.length; },
      click() { return this.onclick?.(); },
      getContext() { return { clearRect() {}, beginPath() {}, arc() {}, fill() {} }; }
    };
  }
  const get = id => { if (!elements.has(id)) { const node = element(); node.id = id; elements.set(id, node); } return elements.get(id); };
  Object.assign(document, { getElementById: get, createElement: element, querySelectorAll: () => [] });
  // Minimal static ancestry/visibility model; actual rendering is covered by the desktop test.
  const html = fs.readFileSync(path.join(__dirname, '../../frontend/index.html'), 'utf8');
  const stack = [], voidTags = new Set(['meta', 'link', 'input', 'br', 'hr', 'img']);
  for (const match of html.matchAll(/<(\/?)([a-z][a-z0-9-]*)\b([^>]*)>/gi)) {
    const [, closing, name, attributes] = match;
    if (closing) { while (stack.length && stack.pop().tagName !== name) {} continue; }
    const id = attributes.match(/\bid="([^"]+)"/)?.[1];
    const node = id ? get(id) : element();
    node.tagName = name;
    for (const c of attributes.match(/\bclass="([^"]+)"/)?.[1].split(/\s+/) || []) node.classList.add(c);
    node.disabled = /\bdisabled\b/.test(attributes);
    stack.at(-1)?.appendChild(node);
    if (!voidTags.has(name)) stack.push(node);
  }
  const context = vm.createContext({ document, window: {}, localStorage: { getItem() { return null; }, setItem() {} }, Blob, atob, btoa,
    addEventListener(event, callback, options) { listeners.push({ event, callback, capture: options === true || options?.capture }); },
    setTimeout(fn, delay) { const id = ++timerId; timers.set(id, { fn, delay }); return id; },
    clearTimeout(id) { timers.delete(id); }, setInterval() {}, requestAnimationFrame() {},
    innerWidth: 1280, innerHeight: 800, location: { reload() {} } });
  for (const name of ['lang.js', 'commands.js', 'organization.js', 'transfer.js', 'app.js']) {
    vm.runInContext(fs.readFileSync(path.join(__dirname, '../../frontend', name), 'utf8'), context);
  }
  context.invoke = async (cmd, args) => {
    if (cmd === 'open_note_editor') return editorView({ note_id: args.params.note_id ?? null });
    if (cmd === 'save_note_editor') return { note_id: args.params.commit ? 'n' : null, protected: !!args.params.new_password, committed: args.params.commit };
    return [];
  };
  return { context, get, document, listeners, timers, run: code => vm.runInContext(code, context),
    async tick(delay) {
      const pending = [...timers].filter(([, timer]) => timer.delay === delay);
      for (const [id] of pending) timers.delete(id);
      await Promise.all(pending.map(([, timer]) => timer.fn()));
    },
    async key(key, options = {}) {
      let stopped = false;
      const event = { key, target: document.activeElement, defaultPrevented: false,
        preventDefault() { this.defaultPrevented = true; }, stopImmediatePropagation() { stopped = true; }, ...options };
      for (const listener of listeners.filter(l => l.event === 'keydown').sort((a, b) => Number(!!b.capture) - Number(!!a.capture))) {
        if (stopped) break;
        await listener.callback(event);
      }
      return event;
    }
  };
}

module.exports = { frontend, editorView };
