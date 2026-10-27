window.SecureNotesOrganization = {
  create(deps) {
    const get = id => document.getElementById(id);
    const tr = (key, values = {}) => Object.entries(values).reduce((text, [name, value]) => text.replace(`{${name}}`, String(value)), deps.t(key));
    const visible = id => !get(id).classList.contains("hidden");
    const focus = node => { if (node && !node.disabled && node.isConnected !== false && (!node.getClientRects || node.getClientRects().length)) { node.focus?.(); return true; } return false; };
    const status = (id, text) => { get(id).textContent = text; get(id).setAttribute("aria-label", text); };
    const button = (label, run, className = "btn small secondary") => {
      const node = document.createElement("button"); node.type = "button"; node.className = className; node.textContent = label; node.onclick = run; return node;
    };
    let folders = [], activeId = null, folderRequest = 0, folderEditEpoch = 0, editingId = null, saving = false;
    let reader = null, readerEpoch = 0, readerOperation = 0, referenceData = null;
    let folderFocus = null, referenceFocus = null;

    function renderFolders() {
      get("foldersList").replaceChildren();
      folders.forEach((folder, index) => {
        const row = document.createElement("div"); row.className = "folder-row" + (folder.id === activeId ? " active" : "");
        const open = button("", () => openFolder(folder.id), "folder-open"); open.id = `folder-open-${index}`;
        const name = document.createElement("span"); name.textContent = folder.name;
        const count = document.createElement("b"); count.textContent = String(folder.count ?? "!");
        open.appendChild(name); open.appendChild(count); open.title = folder.params.query || folder.name;
        const edit = button("✎", () => openFolderEditor(folder.id), "folder-tool"); edit.id = `folder-edit-${index}`;
        edit.setAttribute("aria-label", `${deps.t("folder_edit")} : ${folder.name}`);
        const remove = button("×", () => deleteFolder(folder), "folder-tool"); remove.setAttribute("aria-label", `${deps.t("folder_delete")} : ${folder.name}`);
        row.appendChild(open); row.appendChild(edit); row.appendChild(remove); get("foldersList").appendChild(row);
      });
    }

    async function refreshFolders() {
      if (!deps.state().unlocked) return [];
      const epoch = ++folderRequest;
      try {
        const result = await deps.invoke("list_smart_folders");
        if (epoch !== folderRequest || !deps.state().unlocked) return [];
        folders = result;
        renderFolders();
        status("foldersStatus", folders.length ? "" : deps.t("folders_empty"));
        return folders;
      } catch (error) {
        if (epoch === folderRequest && !await deps.onError(error)) status("foldersStatus", deps.t("folder_failed") + " " + String(error));
        return [];
      }
    }

    async function openFolder(id, { prepare = true } = {}) {
      if (!deps.state().unlocked) return false;
      if (prepare && !await deps.prepareNavigation()) return false;
      const view = deps.beginView();
      status("foldersStatus", deps.t("folder_loading"));
      try {
        const result = await deps.invoke("run_smart_folder", { id });
        if (!deps.isCurrentView(view) || !deps.state().unlocked) return false;
        activeId = id;
        deps.applyFolder(result);
        renderFolders();
        refreshFolders();
        return true;
      } catch (error) {
        if (deps.isCurrentView(view) && !await deps.onError(error)) status("foldersStatus", deps.t("folder_failed") + " " + String(error));
        return false;
      }
    }

    const day = (iso, end = false) => {
      if (!iso) return "";
      const date = new Date(iso); if (end) date.setUTCDate(date.getUTCDate() - 1);
      return date.toISOString().slice(0, 10);
    };

    async function openFolderEditor(id = null) {
      if (saving || !deps.state().unlocked) return;
      const current = deps.currentSearch();
      const previous = document.activeElement;
      if (!await deps.prepareNavigation()) return;
      const epoch = ++folderEditEpoch;
      try {
        if (id && !folders.some(f => f.id === id)) await refreshFolders();
        const profiles = await deps.refreshProfiles();
        if (epoch !== folderEditEpoch || !deps.state().unlocked || deps.commandsOpen()) return;
        const folder = folders.find(f => f.id === id);
        if (id && !folder) throw new Error(deps.t("folder_failed"));
        editingId = id;
        folderFocus = previous;
        const params = folder?.params || current;
        get("folderModalTitle").textContent = deps.t(id ? "folder_edit" : "folder_new");
        get("folderName").value = folder?.name || "";
        get("folderQuery").value = params.query || "";
        get("folderTag").value = params.tag || "";
        get("folderFrom").value = day(params.date_from);
        get("folderTo").value = day(params.date_to, true);
        get("folderProfile").replaceChildren();
        const addProfile = (id, name) => { const option = document.createElement("option"); option.value = id; option.textContent = name; get("folderProfile").appendChild(option); };
        addProfile("", deps.t("folder_all_profiles"));
        profiles.forEach(p => addProfile(p.id, p.name));
        if (params.profile_id && !profiles.some(p => p.id === params.profile_id)) addProfile(params.profile_id, deps.t("folder_deleted_profile"));
        get("folderProfile").value = params.profile_id || "";
        status("folderStatus", "");
        get("folderModal").classList.remove("hidden");
        get("folderName").focus();
      } catch (error) { if (!await deps.onError(error)) await deps.alert(String(error)); }
    }

    function closeFolderEditor(force = false, restore = true) {
      if (saving && !force) return;
      const wasVisible = visible("folderModal"), previous = folderFocus;
      folderFocus = null;
      folderEditEpoch++;
      editingId = null;
      get("folderModal").classList.add("hidden");
      for (const id of ["folderName", "folderQuery", "folderTag", "folderFrom", "folderTo"]) get(id).value = "";
      get("folderProfile").replaceChildren();
      status("folderStatus", "");
      if (restore && wasVisible && deps.state().unlocked && !deps.commandsOpen() && !focus(previous)) focus(get("saveSearchBtn"));
    }

    async function saveFolder() {
      if (saving) return;
      saving = true; get("saveFolderBtn").disabled = true;
      const epoch = folderEditEpoch;
      try {
        const parse = (value, end = false) => { if (!value) return null; const date = new Date(value + "T00:00:00Z"); if (end) date.setUTCDate(date.getUTCDate() + 1); return date.toISOString(); };
        const params = { query: get("folderQuery").value || null, profile_id: get("folderProfile").value || null,
          tag: get("folderTag").value.trim().toLowerCase() || null, date_from: parse(get("folderFrom").value), date_to: parse(get("folderTo").value, true) };
        const folder = await deps.invoke("save_smart_folder", { id: editingId, name: get("folderName").value, params });
        if (epoch !== folderEditEpoch || !deps.state().unlocked) return;
        await refreshFolders();
        if (epoch !== folderEditEpoch) return;
        saving = false; get("saveFolderBtn").disabled = false; closeFolderEditor();
        await openFolder(folder.id, { prepare: false });
      } catch (error) { if (epoch === folderEditEpoch && !await deps.onError(error)) status("folderStatus", String(error)); }
      finally { if (epoch === folderEditEpoch) { saving = false; get("saveFolderBtn").disabled = false; } }
    }

    async function deleteFolder(folder) {
      if (!(await deps.confirm(deps.t("folder_delete_confirm"))).ok || !deps.state().unlocked) return;
      try {
        await deps.invoke("delete_smart_folder", { id: folder.id });
        if (activeId === folder.id) { activeId = null; await deps.onFolderRemoved(); }
        await refreshFolders();
      } catch (error) { if (!await deps.onError(error)) await deps.alert(String(error)); }
    }

    function targetButton(target, operation) {
      const node = button("", () => openTarget(target.id, operation), "context-link");
      const name = document.createElement("strong"); name.textContent = target.is_locked ? deps.t("links_locked") : target.title || deps.t("commands_untitled");
      const snippet = deps.preview(target.preview || "");
      const preview = document.createElement("small"); preview.textContent = `${target.profile_name || ""}${snippet ? " · " + [...snippet].slice(0, 120).join("") : ""}`;
      node.appendChild(name); node.appendChild(preview); node.title = target.is_locked ? deps.t("links_locked") : snippet || target.title;
      return node;
    }

    function referenceRow(reference, operation) {
      const row = document.createElement("div"); row.className = "context-row";
      const label = document.createElement("div"); label.className = "context-label"; label.textContent = reference.label; row.appendChild(label);
      if (reference.status === "missing" || reference.status === "ambiguous" || reference.status === "trashed") {
        const hint = document.createElement("p"); hint.className = "muted"; hint.textContent = deps.t(`links_${reference.status}`); row.appendChild(hint);
      }
      for (const target of reference.targets || []) {
        if (reference.status === "trashed") row.appendChild(button(deps.t("links_restore"), () => restoreTarget(target.id, operation)));
        else row.appendChild(targetButton(target));
      }
      if (reference.can_create && reader && !reader.is_locked) row.appendChild(button(deps.t("links_create"), () => createTarget(reference.reference, operation)));
      return row;
    }

    async function loadReader(note) {
      clearReader(); reader = note;
      const epoch = readerEpoch;
      if (note.is_locked) { status("contextStatus", deps.t("links_private")); return; }
      status("contextStatus", deps.t("links_loading"));
      try {
        const context = await deps.invoke("get_note_context", { id: note.id });
        if (epoch !== readerEpoch || !deps.state().unlocked || deps.state().viewingNote?.id !== note.id) return;
        const incoming = context.incoming || [], outgoing = context.outgoing || [];
        get("incomingTitle").textContent = tr("links_incoming_count", { count: context.incoming_total ?? incoming.length });
        get("outgoingTitle").textContent = tr("links_outgoing_count", { count: context.outgoing_total ?? outgoing.length });
        if (!incoming.length) get("incomingLinks").textContent = deps.t("links_none");
        for (const target of incoming) { const row = document.createElement("div"); row.className = "context-row"; row.appendChild(targetButton(target)); get("incomingLinks").appendChild(row); }
        if (!outgoing.length) get("outgoingLinks").textContent = deps.t("links_none");
        outgoing.forEach(reference => get("outgoingLinks").appendChild(referenceRow(reference)));
        status("contextStatus", context.incoming_total > 50 || context.outgoing_total > 50 ? deps.t("links_limit") : "");
        for (const span of get("vContent").querySelectorAll(".wikilink")) {
          const target = outgoing.find(r => r.reference === span.dataset.ref)?.targets?.[0];
          if (target && !target.is_locked) span.title = `${target.title}\n${target.preview}`;
        }
      } catch (error) { if (epoch === readerEpoch && !await deps.onError(error)) status("contextStatus", deps.t("links_failed") + " " + String(error)); }
    }

    async function openTarget(id, operation = ++readerOperation) {
      try {
        const note = await deps.invoke("get_note", { id });
        if (operation !== readerOperation || !deps.state().unlocked || deps.commandsOpen()) return;
        closeReference(false); await deps.viewNote(note);
      } catch (error) { if (operation === readerOperation && !await deps.onError(error)) status("contextStatus", deps.t("links_failed") + " " + String(error)); }
    }

    async function openReference(reference) {
      if (!reader || !deps.state().unlocked) return;
      const operation = ++readerOperation;
      try {
        const result = await deps.invoke("resolve_note_reference", { reference });
        if (operation !== readerOperation || !deps.state().unlocked || deps.commandsOpen()) return;
        if (result.status === "resolved" || result.status === "locked") { await openTarget(result.targets[0].id, operation); return; }
        referenceData = result;
        referenceFocus = document.activeElement;
        get("referenceTitle").textContent = result.label;
        get("referenceTargets").replaceChildren(referenceRow(result, operation));
        status("referenceStatus", deps.t(`links_${result.status}`));
        get("referenceModal").classList.remove("hidden");
        get("closeReferenceBtn").focus();
      } catch (error) { if (operation === readerOperation && !await deps.onError(error)) status("contextStatus", deps.t("links_failed") + " " + String(error)); }
    }

    async function createTarget(reference, operation = ++readerOperation) {
      if (!reader || reader.is_locked) return;
      const sourceId = reader.id;
      try {
        const id = await deps.invoke("create_note_from_reference", { sourceId, reference });
        if (operation !== readerOperation || !deps.state().unlocked) return;
        closeReference(false); await deps.onNotesChanged();
        if (deps.commandsOpen()) { if (reader) await loadReader(reader); return; }
        await deps.editNote(id);
      } catch (error) { if (operation === readerOperation && !await deps.onError(error)) status(visible("referenceModal") ? "referenceStatus" : "contextStatus", deps.t("links_failed") + " " + String(error)); }
    }

    async function restoreTarget(id, operation = ++readerOperation) {
      try {
        await deps.invoke("restore_note", { id });
        if (operation !== readerOperation || !deps.state().unlocked) return;
        await deps.onNotesChanged();
        if (deps.commandsOpen()) { if (reader) await loadReader(reader); return; }
        await openTarget(id, operation);
      } catch (error) { if (operation === readerOperation && !await deps.onError(error)) status("contextStatus", deps.t("links_failed") + " " + String(error)); }
    }

    function closeReference(restore = true) {
      const previous = referenceFocus, wasVisible = visible("referenceModal"); referenceFocus = null;
      referenceData = null; get("referenceModal").classList.add("hidden"); get("referenceTargets").replaceChildren(); get("referenceTitle").textContent = ""; status("referenceStatus", "");
      if (restore && wasVisible && deps.state().unlocked && !deps.commandsOpen()) focus(previous);
    }
    function clearReader() {
      readerEpoch++; readerOperation++; reader = null; closeReference(false);
      get("incomingLinks").replaceChildren(); get("outgoingLinks").replaceChildren(); status("contextStatus", "");
      get("incomingTitle").textContent = deps.t("links_incoming"); get("outgoingTitle").textContent = deps.t("links_outgoing");
    }

    function keyboard(event) {
      if (visible("sysModal") || deps.commandsOpen()) return;
      const dialog = visible("referenceModal") ? "referenceModal" : visible("folderModal") ? "folderModal" : null;
      if (!dialog) return;
      if (event.key === "Escape") {
        event.preventDefault(); event.stopImmediatePropagation?.(); deps.activity();
        if (dialog === "referenceModal") closeReference(); else closeFolderEditor();
      } else if (dialog === "folderModal" && (event.ctrlKey || event.metaKey) && event.key === "Enter") {
        event.preventDefault(); event.stopImmediatePropagation?.(); deps.activity(); saveFolder();
      } else if (event.key === "Tab") {
        const nodes = [...get(dialog).querySelectorAll("button,input,select")].filter(n => !n.disabled && (!n.getClientRects || n.getClientRects().length));
        const index = nodes.indexOf(document.activeElement);
        if (nodes.length && (index < 0 || (event.shiftKey && index === 0) || (!event.shiftKey && index === nodes.length - 1))) { event.preventDefault(); nodes[event.shiftKey ? nodes.length - 1 : 0].focus(); }
      }
    }

    get("saveSearchBtn").onclick = () => openFolderEditor();
    get("newFolderBtn").onclick = () => openFolderEditor();
    get("cancelFolderBtn").onclick = () => closeFolderEditor();
    get("saveFolderBtn").onclick = saveFolder;
    get("closeReferenceBtn").onclick = () => closeReference();
    get("searchHelpBtn").onclick = () => deps.alert(deps.t("search_help"), deps.t("search_help_title"));
    addEventListener("keydown", keyboard, true);

    return { refreshFolders, openFolder, openFolderEditor, loadReader, clearReader, openReference,
      activeFolderId: () => activeId, leaveFolder() { activeId = null; renderFolders(); },
      isOpen: () => visible("folderModal") || visible("referenceModal"), isSaving: () => saving,
      closeDialogs() { closeFolderEditor(false, false); closeReference(false); },
      clear() {
        folderRequest++; folderEditEpoch++; folders = []; activeId = null; saving = false;
        closeFolderEditor(true, false); clearReader(); get("foldersList").replaceChildren(); status("foldersStatus", ""); get("saveFolderBtn").disabled = false;
      },
    };
  },
};
