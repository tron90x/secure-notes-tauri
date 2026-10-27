// Contrôleur indépendant : aucune donnée de note dans localStorage ou sur disque côté UI.
window.SecureNotesCommands = {
  create(deps) {
    const get = id => document.getElementById(id);
    const tr = (key, values = {}) => Object.entries(values).reduce((text, [name, value]) => text.replace(`{${name}}`, String(value)), deps.t(key));
    const visible = id => !get(id).classList.contains("hidden");
    const status = (id, text) => {
      get(id).textContent = text;
      // WebKitGTK n'expose pas toujours le texte des régions status via AT-SPI.
      get(id).setAttribute("aria-label", text);
    };
    const fold = value => String(value).normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
    const matches = (value, query) => fold(query).split(/\s+/).filter(Boolean).every(word => fold(value).includes(word));
    const focus = node => {
      if (!node || node.disabled || node.isConnected === false || (node.getClientRects && !node.getClientRects().length)) return false;
      node.focus?.();
      return true;
    };
    let vaultEpoch = 0, searchEpoch = 0, searchTimer = null, entries = [], selected = 0;
    let paletteFocus = null, captureFocus = null, executing = false, pendingIntent = null;
    let notePicker = null;
    let captureEpoch = 0, captureSession = null, captureChange = 0, captureSaved = 0, hasDraft = false;
    let captureTimer = null, capturePromise = null, captureOpening = false, captureFinishing = false;

    const actions = [
      ["quick", "command_quick", "⚡", "Ctrl+Shift+N", "capture rapide quick capture"],
      ["new", "command_new", "+", "", "nouvelle note new note"],
      ["drafts", "command_drafts", "📝", "", "brouillon draft"],
      ["dashboard", "command_dashboard", "📊", "", "statistiques stats dashboard"],
      ["graph", "command_graph", "🕸", "", "liens graphe graph"],
      ["backup", "command_backup", "📦", "", "sauvegarder backup"],
      ["backups", "command_backups", "↩", "", "restaurer sauvegardes backups"],
      ["trash", "command_trash", "🗑", "", "corbeille trash"],
      ["search", "command_search", "⌕", "", "recherche filtres dates search"],
      ["folders", "command_folders", "📂", "", "dossiers intelligents folders"],
      ["save_search", "command_save_search", "📂", "", "enregistrer recherche saved search dossier"],
      ["import", "command_import", "⇥", "", "importer markdown texte archive import"],
      ["export", "command_export", "⇤", "", "exporter markdown archive export"],
      ["lock", "command_lock", "🔒", "", "verrouiller lock"],
    ];

    function requireUnlocked(intent) {
      if (deps.state().unlocked) return true;
      pendingIntent = intent;
      deps.requestUnlock(intent);
      return false;
    }

    function queryParts() {
      const text = get("commandQuery").value.trim();
      if (notePicker) return { scope: "note", query: text };
      const scope = { ">": "action", "#": "tag", "@": "profile", "/": "folder" }[text[0]] || "all";
      return { scope, query: scope === "all" ? text : text.slice(1).trim() };
    }

    function actionEntries(query) {
      return actions.filter(([, key, , , aliases]) => matches(`${deps.t(key)} ${aliases}`, query))
        .map(([id, key, icon, shortcut]) => ({ kind: "action", id, label: deps.t(key), detail: shortcut, icon }));
    }

    function renderEntries(items) {
      entries = items;
      selected = 0;
      get("commandResults").replaceChildren();
      items.forEach((item, index) => {
        const button = document.createElement("button");
        button.type = "button";
        button.id = `command-result-${index}`;
        button.className = "command-item";
        button.tabIndex = -1;
        button.setAttribute("role", "option");
        const icon = document.createElement("span");
        icon.className = "command-icon";
        icon.textContent = item.icon;
        icon.setAttribute("aria-hidden", "true");
        const copy = document.createElement("span");
        copy.className = "command-copy";
        const title = document.createElement("strong");
        title.textContent = item.label;
        const detail = document.createElement("small");
        detail.textContent = item.detail;
        copy.appendChild(title);
        copy.appendChild(detail);
        button.appendChild(icon);
        button.appendChild(copy);
        button.onclick = () => runEntry(index);
        button.onmouseenter = () => selectEntry(index, false);
        get("commandResults").appendChild(button);
      });
      selectEntry(0, false);
    }

    function selectEntry(index, scroll = true) {
      selected = entries.length ? (index + entries.length) % entries.length : 0;
      [...get("commandResults").children].forEach((button, i) => button.setAttribute("aria-selected", String(i === selected)));
      const button = get("commandResults").children[selected];
      if (button) {
        get("commandQuery").setAttribute("aria-activedescendant", button.id);
        if (scroll) button.scrollIntoView?.({ block: "nearest" });
      } else get("commandQuery").removeAttribute("aria-activedescendant");
    }

    async function searchPalette(epoch) {
      if (epoch !== searchEpoch || !visible("commandModal") || !deps.state().unlocked) return;
      const { scope, query } = queryParts();
      const local = scope === "all" || scope === "action" ? actionEntries(query) : [];
      renderEntries(local);
      if (scope === "action") {
        status("commandStatus", local.length ? tr("commands_count", { count: local.length }) : deps.t("commands_empty"));
        return;
      }
      status("commandStatus", deps.t("commands_loading"));
      try {
        const found = await deps.invoke("search_command_items", { query });
        if (epoch !== searchEpoch || !visible("commandModal") || !deps.state().unlocked) return;
        const notes = (found.notes || []).map(note => ({
          kind: "note", id: note.id, data: note, icon: note.is_locked ? "🔒" : "📝",
          label: note.is_locked ? deps.t("commands_locked") : note.title || deps.t("commands_untitled"),
          detail: `${deps.t("commands_note")} · ${note.profile_name}`,
        }));
        const profiles = (found.profiles || []).map(profile => ({ kind: "profile", id: profile.id, data: profile, label: profile.name, detail: deps.t("commands_profile"), icon: "👤" }));
        const tags = (found.tags || []).map(([tag, count]) => ({ kind: "tag", id: tag, label: `#${tag}`, detail: tr("commands_tag", { count }), icon: "🏷" }));
        const folders = (found.folders || []).map(folder => ({ kind: "folder", id: folder.id, label: folder.name, detail: deps.t("commands_folder"), icon: "📂" }));
        const items = scope === "note" ? notes : scope === "folder" ? folders : scope === "profile" ? profiles : scope === "tag" ? tags : query ? [...notes, ...folders, ...profiles, ...tags, ...local] : [...local, ...folders, ...notes, ...profiles, ...tags];
        renderEntries(items);
        status("commandStatus", items.length ? tr("commands_count", { count: items.length }) : deps.t("commands_empty"));
      } catch (error) {
        if (epoch !== searchEpoch || !visible("commandModal")) return;
        if (!await deps.onError(error)) status("commandStatus", deps.t("commands_failed") + " " + String(error));
      }
    }

    async function openPalette(picker = null) {
      if (executing || visible("sysModal") || !requireUnlocked("palette")) return;
      if (visible("commandModal")) { focus(get("commandQuery")); return; }
      paletteFocus = visible("quickCaptureModal") ? get(captureSession ? "captureContent" : "captureProfileName") : document.activeElement;
      notePicker = typeof picker === "function" ? picker : null;
      get("commandTitle").textContent = deps.t(notePicker ? "commands_pick_title" : "commands_title");
      get("commandQuery").placeholder = deps.t(notePicker ? "commands_pick_placeholder" : "commands_placeholder");
      get("commandQuery").value = "";
      get("commandModal").classList.remove("hidden");
      focus(get("commandQuery"));
      await searchPalette(++searchEpoch);
    }

    function closePalette(restore = true) {
      const previous = paletteFocus;
      paletteFocus = null;
      clearTimeout(searchTimer);
      searchEpoch++;
      entries = [];
      notePicker = null;
      get("commandTitle").textContent = deps.t("commands_title");
      get("commandQuery").placeholder = deps.t("commands_placeholder");
      get("commandModal").classList.add("hidden");
      get("commandResults").replaceChildren();
      get("commandQuery").value = "";
      get("commandQuery").removeAttribute("aria-activedescendant");
      status("commandStatus", "");
      if (restore && deps.state().unlocked && !focus(previous)) focus(get(visible("quickCaptureModal") ? "captureContent" : "commandsBtn"));
    }

    async function runEntry(index) {
      const item = entries[index];
      if (!item || executing || !deps.state().unlocked) return;
      const epoch = vaultEpoch;
      const picker = notePicker;
      executing = true;
      try {
        if (picker && item.kind === "note") { closePalette(false); if (epoch === vaultEpoch) await picker(item.data); return; }
        if (item.kind === "action" && item.id === "quick") { await openCapture(); return; }
        closePalette(false);
        if (item.kind === "action" && (item.id === "import" || item.id === "export")) { await deps.runAction(item.id); return; }
        if (item.kind === "action" && item.id === "lock") { await deps.runAction("lock"); return; }
        if (!await finishCapture(false) || !await deps.prepareNavigation()) return;
        if (epoch !== vaultEpoch || !deps.state().unlocked) return;
        if (item.kind === "action") await deps.runAction(item.id);
        else await deps.navigate(item);
      } catch (error) { if (epoch === vaultEpoch) await deps.onCommandError(error); }
      finally { if (epoch === vaultEpoch) executing = false; }
    }

    function captureControls() {
      const disabled = !captureSession || captureOpening || captureFinishing;
      for (const id of ["captureTitle", "captureContent", "captureFullBtn", "saveCaptureBtn"]) get(id).disabled = disabled;
      get("captureProfile").disabled = captureOpening || captureFinishing || !get("captureProfile").children.length;
      get("captureProfileName").disabled = captureOpening || captureFinishing;
      get("createCaptureProfileBtn").disabled = captureOpening || captureFinishing;
    }

    function captureProfiles(profiles, selectedId) {
      get("captureProfile").replaceChildren();
      profiles.forEach(profile => {
        const option = document.createElement("option");
        option.value = profile.id;
        option.textContent = profile.name;
        get("captureProfile").appendChild(option);
      });
      get("captureProfile").value = profiles.some(p => p.id === selectedId) ? selectedId : profiles[0]?.id || "";
      get("captureProfileSetup").classList.toggle("hidden", profiles.length > 0);
    }

    async function createCaptureSession(epoch) {
      if (!get("captureProfile").value) {
        status("captureStatus", deps.t("capture_need_profile"));
        return;
      }
      const session = await deps.invoke("open_note_editor", { params: { profile_id: get("captureProfile").value } });
      if (epoch !== captureEpoch || !deps.state().unlocked) {
        await deps.invoke("close_note_editor", { token: session.token });
        return;
      }
      captureSession = session;
      status("captureStatus", deps.t("capture_ready"));
    }

    async function openCapture() {
      if (visible("sysModal") || !requireUnlocked("capture")) return;
      if (visible("quickCaptureModal")) { focus(get(captureSession ? "captureContent" : "captureProfileName")); return; }
      captureFocus = visible("commandModal") ? paletteFocus : document.activeElement;
      closePalette(false);
      const epoch = ++captureEpoch;
      captureOpening = true;
      captureChange = captureSaved = 0;
      hasDraft = false;
      get("captureTitle").value = get("captureContent").value = get("captureProfileName").value = "";
      get("quickCaptureModal").classList.remove("hidden");
      status("captureStatus", deps.t("capture_opening"));
      captureControls();
      try {
        const profiles = await deps.refreshProfiles();
        if (epoch !== captureEpoch || !deps.state().unlocked) return;
        captureProfiles(profiles, deps.state().currentProfileId);
        await createCaptureSession(epoch);
      } catch (error) {
        if (epoch === captureEpoch && !await deps.onError(error)) status("captureStatus", deps.t("capture_error") + " " + String(error));
      } finally {
        if (epoch === captureEpoch) {
          captureOpening = false;
          captureControls();
          if (!visible("commandModal") && !visible("sysModal")) focus(get(captureSession ? "captureContent" : "captureProfileName"));
        }
      }
    }

    function meaningfulCapture() { return !!(get("captureTitle").value.trim() || get("captureContent").value.trim()); }

    function captureSnapshot(commit = false) {
      const body = get("captureContent").value;
      const title = get("captureTitle").value.trim() || [...(body.split(/\r?\n/).find(line => line.trim()) || deps.t("capture_untitled")).trim()].slice(0, 80).join("");
      return { token: captureSession.token, profile_id: get("captureProfile").value,
        content: { title, content: body, tags: [], attachment_name: null, attachment_data: null, wallpaper: "aurora", glass: true },
        new_password: null, remove_protection: false, commit };
    }

    function markCaptureDirty() {
      if (!captureSession || captureFinishing) return;
      captureChange++;
      status("captureStatus", deps.t("capture_dirty"));
      clearTimeout(captureTimer);
      captureTimer = setTimeout(() => flushCapture(false), 1200);
    }

    async function flushCapture(publish) {
      const epoch = captureEpoch;
      while (capturePromise) {
        const pending = capturePromise;
        await pending;
        if (capturePromise === pending) capturePromise = null;
        if (epoch !== captureEpoch) return false;
      }
      if (!visible("quickCaptureModal")) return true;
      if (captureOpening) return false;
      if (publish && !captureSession) { status("captureStatus", deps.t("capture_need_profile")); return false; }
      if (publish && !meaningfulCapture()) { status("captureStatus", deps.t("capture_empty")); return false; }
      if (!captureSession) return !meaningfulCapture();
      if (!hasDraft && !meaningfulCapture() && !publish) return true;
      if (captureChange === captureSaved && !publish) return true;
      const sequence = captureChange;
      const snapshot = captureSnapshot(publish);
      status("captureStatus", deps.t("capture_saving"));
      const task = (async () => {
        try {
          const result = await deps.invoke("save_note_editor", { params: snapshot });
          if (epoch !== captureEpoch || !captureSession) return false;
          captureSaved = sequence;
          hasDraft = !result.committed;
          captureSession.note_id = result.note_id;
          captureSession.profile_id = snapshot.profile_id;
          status("captureStatus", deps.t(captureChange > captureSaved ? "capture_dirty" : "capture_saved"));
          deps.onDraftsChanged();
          return true;
        } catch (error) {
          if (epoch === captureEpoch && !await deps.onError(error)) status("captureStatus", deps.t("capture_error") + " " + String(error));
          return false;
        }
      })();
      capturePromise = task;
      const success = await task;
      if (capturePromise === task) capturePromise = null;
      if (epoch === captureEpoch && captureChange > sequence && !captureFinishing) {
        clearTimeout(captureTimer);
        captureTimer = setTimeout(() => flushCapture(false), 1200);
      }
      return success;
    }

    function resetCapture(restore) {
      const previous = captureFocus;
      captureFocus = null;
      clearTimeout(captureTimer);
      captureEpoch++;
      capturePromise = null;
      captureSession = null;
      captureOpening = captureFinishing = false;
      captureChange = captureSaved = 0;
      hasDraft = false;
      get("quickCaptureModal").classList.add("hidden");
      for (const id of ["captureTitle", "captureContent", "captureProfileName"]) get(id).value = "";
      get("captureProfile").replaceChildren();
      status("captureStatus", "");
      captureControls();
      if (restore && deps.state().unlocked && !visible("commandModal") && !focus(previous)) focus(get("quickCaptureBtn"));
    }

    async function finishCapture(publish = false, transfer = false) {
      if (!visible("quickCaptureModal")) return true;
      if (captureFinishing) return false;
      if (captureOpening) {
        if (!publish && !transfer) { resetCapture(true); return true; }
        return false;
      }
      captureFinishing = true;
      captureControls();
      clearTimeout(captureTimer);
      const epoch = captureEpoch;
      try {
        if (!await flushCapture(publish)) return false;
        if (epoch !== captureEpoch) return false;
        const session = captureSession;
        const draftId = hasDraft ? session?.draft_id : null;
        const profileId = get("captureProfile").value;
        if (session) await deps.invoke("close_note_editor", { token: session.token });
        if (epoch !== captureEpoch) return false;
        resetCapture(!transfer);
        deps.onDraftsChanged();
        if (transfer) await deps.openFullEditor(draftId, profileId);
        else if (publish) await deps.onCaptured(profileId);
        return true;
      } catch (error) {
        if (epoch === captureEpoch && !await deps.onError(error)) status("captureStatus", deps.t("capture_error") + " " + String(error));
        return false;
      } finally {
        if (epoch === captureEpoch) { captureFinishing = false; captureControls(); }
      }
    }

    async function createProfile() {
      if (captureOpening || captureFinishing) return;
      const name = get("captureProfileName").value.trim();
      if (!name) { focus(get("captureProfileName")); return; }
      const epoch = captureEpoch;
      captureOpening = true;
      captureControls();
      try {
        const profile = await deps.createProfile(name);
        const profiles = await deps.refreshProfiles();
        if (epoch !== captureEpoch) return;
        captureProfiles(profiles, profile.id);
        await createCaptureSession(epoch);
      } catch (error) {
        if (epoch === captureEpoch && !await deps.onError(error)) status("captureStatus", deps.t("capture_error") + " " + String(error));
      } finally {
        if (epoch === captureEpoch) {
          captureOpening = false;
          captureControls();
          if (!visible("commandModal") && !visible("sysModal")) focus(get(captureSession ? "captureContent" : "captureProfileName"));
        }
      }
    }

    function trapTab(event, dialog) {
      const nodes = [...get(dialog).querySelectorAll('button,input,textarea,select,[tabindex="0"]')]
        .filter(node => !node.disabled && (node.tabIndex ?? 0) >= 0 && (!node.getClientRects || node.getClientRects().length));
      if (!nodes.length) return;
      const index = nodes.indexOf(document.activeElement);
      if (index < 0 || (!event.shiftKey && index === nodes.length - 1) || (event.shiftKey && index === 0)) {
        event.preventDefault();
        focus(nodes[event.shiftKey ? nodes.length - 1 : 0]);
      }
    }

    function keyboard(event) {
      if (event.isComposing || event.repeat || visible("sysModal")) return;
      const key = event.key.toLowerCase();
      const modifier = event.ctrlKey || event.metaKey;
      let action = null;
      if (modifier && !event.altKey && !event.shiftKey && key === "k") action = () => visible("commandModal") ? closePalette() : openPalette();
      else if (modifier && !event.altKey && event.shiftKey && key === "n") action = openCapture;
      else if (visible("commandModal")) {
        if (key === "escape") action = closePalette;
        else if (key === "arrowdown" && event.target === get("commandQuery")) action = () => selectEntry(selected + 1);
        else if (key === "arrowup" && event.target === get("commandQuery")) action = () => selectEntry(selected - 1);
        else if (key === "enter" && event.target === get("commandQuery")) action = () => runEntry(selected);
        else if (key === "tab") { trapTab(event, "commandModal"); return; }
      } else if (visible("quickCaptureModal")) {
        if (key === "escape") action = () => finishCapture(false);
        else if (modifier && key === "enter") action = () => finishCapture(true);
        else if (key === "tab") { trapTab(event, "quickCaptureModal"); return; }
      }
      if (action) {
        event.preventDefault();
        event.stopImmediatePropagation?.();
        deps.activity?.();
        return action();
      }
    }

    get("commandsBtn").onclick = openPalette;
    get("closeCommandsBtn").onclick = () => closePalette();
    get("quickCaptureBtn").onclick = openCapture;
    get("closeCaptureBtn").onclick = () => finishCapture(false);
    get("saveCaptureBtn").onclick = () => finishCapture(true);
    get("captureFullBtn").onclick = () => finishCapture(false, true);
    get("createCaptureProfileBtn").onclick = createProfile;
    get("captureProfileName").onkeydown = event => { if (event.key === "Enter") { event.preventDefault(); createProfile(); } };
    get("captureTitle").oninput = get("captureContent").oninput = markCaptureDirty;
    get("captureProfile").onchange = markCaptureDirty;
    get("commandQuery").oninput = () => {
      clearTimeout(searchTimer);
      const epoch = ++searchEpoch;
      const { scope, query } = queryParts();
      renderEntries(scope === "all" || scope === "action" ? actionEntries(query) : []);
      status("commandStatus", deps.t("commands_loading"));
      searchTimer = setTimeout(() => searchPalette(epoch), 150);
    };
    get("commandModal").onclick = event => { if (event.target === get("commandModal")) closePalette(); };
    get("quickCaptureModal").onclick = event => { if (event.target === get("quickCaptureModal")) finishCapture(false); };
    addEventListener("keydown", keyboard, true);

    return {
      openPalette, closePalette, openCapture, finishCapture, pickNote: callback => openPalette(callback),
      isOpen: () => visible("commandModal") || visible("quickCaptureModal"),
      hasCapture: () => visible("quickCaptureModal"),
      snapshotForLock: () => captureSession && captureChange > captureSaved && (meaningfulCapture() || hasDraft) ? captureSnapshot(false) : null,
      async onUnlocked() {
        const intent = pendingIntent;
        pendingIntent = null;
        if (intent === "capture") await openCapture();
        else if (intent === "palette") await openPalette();
      },
      clear({ release = true } = {}) {
        vaultEpoch++;
        executing = false;
        pendingIntent = null;
        closePalette(false);
        const token = captureSession?.token;
        resetCapture(false);
        if (token && release) deps.invoke("close_note_editor", { token }).catch(() => {});
      },
    };
  },
};
