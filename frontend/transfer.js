window.SecureNotesTransfer = {
  create(deps) {
    const get = id => document.getElementById(id);
    const visible = () => !get("transferModal").classList.contains("hidden");
    const tr = (key, values = {}) => Object.entries(values).reduce((text, [name, value]) => text.replace(`{${name}}`, String(value)), deps.t(key));
    const status = text => { get("transferStatus").textContent = text; get("transferStatus").setAttribute("aria-label", text); };
    const previewStatus = text => { get("transferPreviewSummary").textContent = text; get("transferPreviewSummary").setAttribute("aria-label", text); };
    let epoch = 0, busy = false, mode = "export", notes = [], files = [], preview = null;
    const selected = new Set(), urls = new Set();

    function filtered() {
      const query = get("transferFilter").value.trim().toLowerCase(), profile = get("transferProfile").value;
      return notes.filter(note => (!profile || profile === note.profile_id) && (!query || `${note.is_locked ? deps.t("commands_locked") : note.title} ${note.profile_name}`.toLowerCase().includes(query)));
    }

    function renderNotes() {
      const plain = get("transferFormat").value !== "encrypted";
      get("transferNotes").replaceChildren();
      filtered().forEach((note, i) => {
        const label = document.createElement("label"); label.className = "transfer-note";
        const checkbox = document.createElement("input"); checkbox.type = "checkbox"; checkbox.id = `transfer-note-${i}`;
        checkbox.checked = selected.has(note.id); checkbox.disabled = busy || (plain && note.is_locked);
        checkbox.onchange = () => { if (checkbox.checked) selected.add(note.id); else selected.delete(note.id); controls(); };
        const text = document.createElement("span"), title = document.createElement("strong"), detail = document.createElement("small");
        title.textContent = note.is_locked ? deps.t("commands_locked") : note.title || deps.t("commands_untitled");
        detail.textContent = `${note.profile_name}${note.legacy ? " · " + deps.t("transfer_legacy") : ""}`;
        if (plain && note.is_locked) label.title = deps.t("transfer_locked_format");
        text.appendChild(title); text.appendChild(detail); label.appendChild(checkbox); label.appendChild(text); get("transferNotes").appendChild(label);
      });
      get("transferSelection").textContent = tr("transfer_selection", { selected: selected.size, total: notes.length });
    }

    function controls() {
      for (const id of ["closeTransferBtn", "transferExportTab", "transferImportTab", "transferFormat", "transferPassword", "transferConfirm", "transferProfile", "transferFilter", "transferSelectAll", "transferSelectNone", "pickImportFilesBtn", "pickImportFolderBtn", "transferFiles", "transferImportPassword", "transferPreserveProfiles"]) get(id).disabled = busy;
      get("exportNotesBtn").disabled = busy || !selected.size;
      get("prepareImportBtn").disabled = busy || !files.length;
      get("transferDestination").disabled = busy || get("transferPreserveProfiles").checked;
      get("transferCopies").disabled = busy || !preview || preview.protected_count > 0;
      get("commitImportBtn").disabled = busy || !preview;
      renderNotes();
    }

    function setMode(next) {
      if (busy) return;
      mode = next;
      get("transferPassword").value = get("transferConfirm").value = get("transferImportPassword").value = "";
      get("transferExportPane").classList.toggle("hidden", mode !== "export");
      get("transferImportPane").classList.toggle("hidden", mode !== "import");
      get("transferExportTab").setAttribute("aria-selected", String(mode === "export"));
      get("transferImportTab").setAttribute("aria-selected", String(mode === "import"));
      for (const [id, active] of [["transferExportTab", mode === "export"], ["transferImportTab", mode === "import"]]) {
        get(id).classList.toggle("secondary", active); get(id).classList.toggle("ghost", !active);
      }
      status("");
    }

    function profilesOptions(profiles, selectedId) {
      for (const id of ["transferProfile", "transferDestination"]) {
        get(id).replaceChildren();
        if (id === "transferProfile") { const all = document.createElement("option"); all.value = ""; all.textContent = deps.t("folder_all_profiles"); get(id).appendChild(all); }
        profiles.forEach(profile => { const option = document.createElement("option"); option.value = profile.id; option.textContent = profile.name; get(id).appendChild(option); });
        get(id).value = profiles.some(p => p.id === selectedId) ? selectedId : id === "transferProfile" ? "" : profiles[0]?.id || "";
      }
    }

    async function refreshNotes() { notes = await deps.invoke("list_transfer_notes"); }

    async function open(next = "export") {
      if (busy || !deps.state().unlocked) return;
      const scope = deps.selection();
      if (!await deps.prepareNavigation()) return;
      const own = ++epoch; busy = true;
      get("transferModal").classList.remove("hidden");
      get("transferFormat").value = "encrypted";
      get("transferPasswords").classList.remove("hidden");
      get("transferPreserveProfiles").checked = true; get("transferCopies").checked = false;
      controls(); status(deps.t("transfer_loading"));
      try {
        const [profiles, result] = await Promise.all([deps.refreshProfiles(), deps.invoke("list_transfer_notes")]);
        if (own !== epoch || !deps.state().unlocked) return;
        notes = result; profilesOptions(profiles, deps.state().currentProfileId);
        selected.clear();
        for (const note of notes) if (!scope.length || scope.includes(note.id)) selected.add(note.id);
        busy = false; setMode(next);
        if (!notes.length && next === "export") status(deps.t("transfer_empty"));
        if (!deps.commandsOpen() && !deps.organizationOpen()) get(next === "import" ? "pickImportFilesBtn" : "transferFormat").focus();
      } catch (error) { if (own === epoch && !await deps.onError(error)) status(deps.t("transfer_failed") + " " + String(error)); }
      finally { if (own === epoch) { busy = false; controls(); } }
    }

    function dropPreview() {
      const token = preview?.token; preview = null;
      if (token) deps.invoke("discard_note_import", { token }).catch(() => {});
      get("transferPreview").replaceChildren(); previewStatus(""); get("transferCopies").checked = false;
    }

    function selectFiles(event) {
      if (busy) return;
      dropPreview(); files = [...event.target.files];
      const bytes = files.reduce((total, file) => total + file.size, 0);
      if (files.length > 4096 || bytes > 96 * 1024 * 1024) { files = []; get("transferFileNames").textContent = ""; status(deps.t("transfer_too_large")); }
      else { status(""); get("transferFileNames").textContent = files.map(file => file.webkitRelativePath || file.name).join(" · "); }
      controls();
    }

    async function encodeFiles(own) {
      const encoded = [];
      for (const file of files) {
        if (own !== epoch || !deps.state().unlocked) return null;
        const bytes = new Uint8Array(await file.arrayBuffer());
        if (own !== epoch || !deps.state().unlocked) return null;
        let binary = "";
        for (let i = 0; i < bytes.length; i += 32768) binary += String.fromCharCode(...bytes.subarray(i, i + 32768));
        encoded.push({ name: file.webkitRelativePath || file.name, data: btoa(binary) });
      }
      return encoded;
    }

    function showPreview(result) {
      preview = result; get("transferCopies").checked = false;
      previewStatus(tr("transfer_preview", { notes: result.notes_count, protected: result.protected_count, attachments: result.attachments_count, duplicates: result.duplicates }));
      get("transferPreview").replaceChildren();
      for (const note of result.notes || []) {
        const row = document.createElement("div"); row.className = "transfer-note";
        row.textContent = `${note.is_locked ? deps.t("commands_locked") : note.title || deps.t("commands_untitled")} · ${note.profile_name}`;
        get("transferPreview").appendChild(row);
      }
      status(tr("transfer_details", { ignored: result.ignored_files, bundles: result.resource_bundles, edited: result.edited_files })
        + (result.notes_count > (result.notes || []).length ? " · " + tr("transfer_preview_limit", { count: (result.notes || []).length }) : ""));
    }

    async function prepareImport() {
      if (busy) return;
      if (!files.length) { status(deps.t("transfer_need_files")); return; }
      const password = get("transferImportPassword").value || null;
      if (files.some(f => f.name.toLowerCase().endsWith(".snotetransfer")) && !password) { status(deps.t("transfer_need_password")); return; }
      dropPreview(); busy = true; controls(); status(deps.t("transfer_busy"));
      get("transferImportPassword").value = "";
      const own = epoch;
      try {
        const input = await encodeFiles(own);
        if (!input || own !== epoch || !deps.state().unlocked) return;
        const result = await deps.invoke("prepare_note_import", { files: input, password });
        if (own !== epoch || !deps.state().unlocked) { deps.invoke("discard_note_import", { token: result.token }).catch(() => {}); return; }
        showPreview(result);
      } catch (error) { if (own === epoch && !await deps.onError(error)) status(deps.t("transfer_failed") + " " + String(error)); }
      finally { if (own === epoch) { busy = false; controls(); } }
    }

    async function chooseDirectory() {
      if (busy) return;
      get("transferImportPassword").value = "";
      const own = epoch; busy = true; controls(); status(deps.t("transfer_busy"));
      try {
        const result = await deps.invoke("choose_directory_import", { title: deps.t("transfer_folder_title") });
        if (own !== epoch || !deps.state().unlocked) { if (result?.token) deps.invoke("discard_note_import", { token: result.token }).catch(() => {}); return; }
        if (!result) { status(""); return; }
        dropPreview(); files = []; get("transferFiles").value = "";
        get("transferFileNames").textContent = result.source || deps.t("transfer_pick_folder");
        showPreview(result);
      } catch (error) {
        if (own === epoch && !await deps.onError(error)) { dropPreview(); status(deps.t("transfer_failed") + " " + String(error)); }
      } finally { if (own === epoch) { busy = false; controls(); } }
    }

    async function commitImport() {
      if (busy || !preview) return;
      const own = epoch; busy = true; controls(); status(deps.t("transfer_busy"));
      try {
        const result = await deps.invoke("commit_note_import", { token: preview.token, destinationProfileId: get("transferDestination").value || null,
          preserveProfiles: get("transferPreserveProfiles").checked, copies: get("transferCopies").checked });
        if (own !== epoch || !deps.state().unlocked) return;
        preview = null; files = []; get("transferFiles").value = "";
        get("transferFileNames").textContent = ""; get("transferPreview").replaceChildren(); previewStatus("");
        status(tr("transfer_import_done", { added: result.added, skipped: result.skipped }));
        await deps.onImported(result);
        if (own !== epoch || !deps.state().unlocked) return;
        const profiles = await deps.refreshProfiles(); await refreshNotes();
        if (own === epoch) profilesOptions(profiles, result.profile_id || deps.state().currentProfileId);
      } catch (error) { if (own === epoch && !await deps.onError(error)) status(deps.t("transfer_failed") + " " + String(error)); }
      finally { if (own === epoch) { busy = false; controls(); } }
    }

    function download(artifact) {
      const bytes = Uint8Array.from(atob(artifact.data), char => char.charCodeAt(0));
      const url = URL.createObjectURL(new Blob([bytes], { type: artifact.mime })); urls.add(url);
      const anchor = document.createElement("a"); anchor.href = url; anchor.download = artifact.name.split(/[/\\]/).pop(); anchor.click();
      setTimeout(() => { URL.revokeObjectURL(url); urls.delete(url); }, 1500);
    }

    async function exportNotes() {
      if (busy) return;
      if (!selected.size) { status(deps.t("transfer_need_selection")); return; }
      if (selected.size > 1000) { status(deps.t("transfer_selection_limit")); return; }
      const format = get("transferFormat").value, password = format === "encrypted" ? get("transferPassword").value : null;
      if (format === "encrypted") {
        if ([...password].length < 8) { status(deps.t("transfer_password_short")); return; }
        if (password !== get("transferConfirm").value) { status(deps.t("transfer_password_mismatch")); return; }
      }
      busy = true; controls(); status(deps.t("transfer_busy"));
      get("transferPassword").value = get("transferConfirm").value = "";
      const own = epoch;
      try {
        const artifact = await deps.invoke("export_notes", { ids: [...selected], format, password });
        if (own !== epoch || !deps.state().unlocked) return;
        download(artifact); status(tr("transfer_export_done", { count: artifact.notes_count }));
      } catch (error) { if (own === epoch && !await deps.onError(error)) status(deps.t("transfer_failed") + " " + String(error)); }
      finally { if (own === epoch) { busy = false; controls(); } }
    }

    function clear({ release = true } = {}) {
      epoch++; const token = preview?.token; preview = null; notes = []; files = []; selected.clear(); busy = false;
      for (const id of ["transferPassword", "transferConfirm", "transferImportPassword", "transferFilter", "transferFiles"]) get(id).value = "";
      for (const id of ["transferNotes", "transferPreview", "transferProfile", "transferDestination"]) get(id).replaceChildren();
      get("transferFileNames").textContent = get("transferSelection").textContent = ""; previewStatus(""); status("");
      get("transferCopies").checked = false; get("transferPreserveProfiles").checked = true;
      get("transferModal").classList.add("hidden");
      for (const url of urls) URL.revokeObjectURL(url); urls.clear();
      if (token && release) deps.invoke("discard_note_import", { token }).catch(() => {});
      controls();
    }

    function close() { if (busy) return false; clear(); return true; }

    get("transferBtn").onclick = () => open("export");
    get("closeTransferBtn").onclick = close;
    get("transferExportTab").onclick = () => setMode("export"); get("transferImportTab").onclick = () => setMode("import");
    get("transferFilter").oninput = get("transferProfile").onchange = renderNotes;
    get("transferFormat").onchange = () => {
      const plain = get("transferFormat").value !== "encrypted"; let removed = 0;
      if (plain) for (const note of notes) if (note.is_locked && selected.delete(note.id)) removed++;
      get("transferPasswords").classList.toggle("hidden", plain);
      get("transferPassword").value = get("transferConfirm").value = "";
      status(removed ? tr("transfer_plain_removed", { count: removed }) : plain ? deps.t("transfer_locked_format") : ""); controls();
    };
    get("transferSelectAll").onclick = () => { for (const note of filtered()) if (get("transferFormat").value === "encrypted" || !note.is_locked) selected.add(note.id); controls(); };
    get("transferSelectNone").onclick = () => { selected.clear(); controls(); };
    get("exportNotesBtn").onclick = exportNotes;
    get("pickImportFilesBtn").onclick = () => get("transferFiles").click(); get("pickImportFolderBtn").onclick = chooseDirectory;
    get("transferFiles").onchange = selectFiles;
    get("prepareImportBtn").onclick = prepareImport; get("commitImportBtn").onclick = commitImport;
    get("transferPreserveProfiles").onchange = controls;
    addEventListener("keydown", event => {
      if (!visible() || !get("sysModal").classList.contains("hidden") || deps.commandsOpen() || deps.organizationOpen()) return;
      if (event.key === "Escape") { event.preventDefault(); event.stopImmediatePropagation?.(); deps.activity(); close(); }
      else if ((event.ctrlKey || event.metaKey) && event.key === "Enter") { event.preventDefault(); event.stopImmediatePropagation?.(); deps.activity(); if (mode === "export") exportNotes(); else if (preview) commitImport(); else prepareImport(); }
      else if (event.key === "Tab") {
        const nodes = [...get("transferModal").querySelectorAll("button,input,select")].filter(n => !n.disabled && (!n.getClientRects || n.getClientRects().length));
        const index = nodes.indexOf(document.activeElement);
        if (nodes.length && (index < 0 || (event.shiftKey && index === 0) || (!event.shiftKey && index === nodes.length - 1))) { event.preventDefault(); nodes[event.shiftKey ? nodes.length - 1 : 0].focus(); }
      }
    }, true);
    return { open, close, clear, isOpen: visible, isBusy: () => busy };
  },
};
