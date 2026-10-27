#!/usr/bin/env python3
"""Smoke test of the real Linux Tauri window over AT-SPI and X11.

Uses an isolated HOME/XDG profile and keeps logs/screenshots in a temporary
directory. Requires python3-pyatspi, python3-xlib and GTK 3 introspection.
"""

import argparse
import ctypes
import json
import zipfile
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

import pyatspi
from Xlib import X, XK, display, protocol


class XkbState(ctypes.Structure):
    _fields_ = [("group", ctypes.c_ubyte), ("locked_group", ctypes.c_ubyte),
                ("base_group", ctypes.c_ushort), ("latched_group", ctypes.c_ushort),
                ("mods", ctypes.c_ubyte), ("base_mods", ctypes.c_ubyte),
                ("latched_mods", ctypes.c_ubyte), ("locked_mods", ctypes.c_ubyte),
                ("compat_state", ctypes.c_ubyte), ("grab_mods", ctypes.c_ubyte),
                ("compat_grab_mods", ctypes.c_ubyte), ("lookup_mods", ctypes.c_ubyte),
                ("compat_lookup_mods", ctypes.c_ubyte), ("ptr_buttons", ctypes.c_ushort)]


class DesktopSmoke:
    def __init__(self, binary):
        self.binary = Path(binary).resolve()
        temp_root = Path(os.environ.get("SECURENOTES_TEST_TMP", "/tmp/opencode"))
        temp_root.mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix="securenotes-desktop-", dir=temp_root))
        for name in ["config", "Documents", "Downloads", "tmp"]:
            (self.root / name).mkdir()
        (self.root / "config/user-dirs.dirs").write_text(
            f'XDG_DOCUMENTS_DIR="{self.root}/Documents"\n'
            f'XDG_DOWNLOAD_DIR="{self.root}/Downloads"\n'
        )
        self.env = os.environ.copy()
        self.env["XAUTHORITY"] = self.env.get("XAUTHORITY", str(Path.home() / ".Xauthority"))
        self.env.update(HOME=str(self.root), XDG_CONFIG_HOME=str(self.root / "config"),
                        XDG_DATA_HOME=str(self.root / "data"), XDG_CACHE_HOME=str(self.root / "cache"),
                        LC_ALL="C.UTF-8", GTK_MODULES="atk-bridge", GTK_USE_PORTAL="0",
                        TMPDIR=str(self.root / "tmp"), APPIMAGE_EXTRACT_AND_RUN="1")
        self.env.pop("NO_AT_BRIDGE", None)
        self.x = display.Display()
        self.keyboard = ctypes.CDLL("libX11.so.6")
        self.keyboard.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.keyboard.XOpenDisplay.restype = ctypes.c_void_p
        self.keyboard.XkbGetState.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.POINTER(XkbState)]
        self.keyboard.XkbKeycodeToKeysym.argtypes = [ctypes.c_void_p, ctypes.c_ubyte, ctypes.c_int, ctypes.c_int]
        self.keyboard.XkbKeycodeToKeysym.restype = ctypes.c_ulong
        self.keyboard.XkbLockGroup.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_uint]
        self.keyboard.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self.keyboard_display = self.keyboard.XOpenDisplay(None)
        assert self.keyboard_display, "Cannot read the active X11 keyboard layout"
        self.initial_group = self.keyboard_group()
        self.keymap = {}
        self.process = None
        self.app_pid = None
        self.log = None
        self.checks = []
        self.vault_password = "desktop-vault-password"
        self.note_password = "desktop-note-password"
        self.new_password = "desktop-new-password"

    def wait(self, callback, label, timeout=25):
        end = time.monotonic() + timeout
        last_error = None
        while time.monotonic() < end:
            try:
                result = callback()
                if result:
                    return result
            except (RuntimeError, LookupError, AttributeError) as error:
                last_error = error
            time.sleep(.15)
        raise AssertionError(f"Timeout: {label}; last error: {last_error}")

    def app(self):
        if not self.process:
            return None
        for app in pyatspi.Registry.getDesktop(0):
            if app and self.owns_process(app.get_process_id()):
                self.app_pid = app.get_process_id()
                return app
        return None

    def owns_process(self, pid):
        # AppImage runtimes can launch AppRun as a child rather than exec it.
        for _ in range(16):
            if pid == self.process.pid:
                return True
            if pid <= 1:
                return False
            try:
                status = Path(f"/proc/{pid}/status").read_text()
                pid = int(next(line.split()[1] for line in status.splitlines() if line.startswith("PPid:")))
            except (OSError, StopIteration, ValueError):
                return False
        return False

    def walk(self, node=None):
        if node is None:
            node = self.app()
        if node is None:
            return
        yield node
        for child in node:
            if child:
                yield from self.walk(child)

    def find(self, element_id):
        if element_id == "draftCount":
            element_id = "draftsBtn"
        return next((n for n in self.walk() if f"id:{element_id}" in n.getAttributes()), None)

    def by_id(self, element_id):
        return self.wait(lambda: self.find(element_id), element_id)

    def text(self, node):
        if isinstance(node, str):
            node = self.find(node)
        if not node:
            return ""
        node.clear_cache()
        if node.getRoleName() == "status bar" and node.name:
            return node.name
        try:
            text = node.queryText().getText(0, -1)
            if text:
                return text
        except (NotImplementedError, RuntimeError):
            pass
        return " ".join(filter(None, [node.name, *(self.text(c) for c in node if c)]))

    def expect_text(self, element_id, expected, timeout=25):
        return self.wait(lambda: expected in self.text(element_id), f"{element_id}: {expected}", timeout)

    def window(self, prefer_native=False):
        root = self.x.screen().root
        prop = root.get_full_property(self.x.intern_atom("_NET_CLIENT_LIST"), X.AnyPropertyType)
        matches = []
        for xid in prop.value if prop else []:
            window = self.x.create_resource_object("window", xid)
            pid = window.get_full_property(self.x.intern_atom("_NET_WM_PID"), X.AnyPropertyType)
            if pid is not None and int(pid.value[0]) == self.app_pid:
                matches.append(window)
        if matches:
            return matches[-1] if prefer_native else matches[0]
        raise LookupError("Native application window not found")

    def focus(self, prefer_native=False):
        window = self.window(prefer_native)
        event = protocol.event.ClientMessage(window=window, client_type=self.x.intern_atom("_NET_ACTIVE_WINDOW"), data=(32, [1, X.CurrentTime, 0, 0, 0]))
        self.x.screen().root.send_event(event, event_mask=X.SubstructureRedirectMask | X.SubstructureNotifyMask)
        self.x.flush()
        time.sleep(.1)

    def click(self, node):
        if isinstance(node, str):
            node = self.by_id(node)
        node.clear_cache()
        component = node.queryComponent()
        try:
            component.scrollTo(pyatspi.SCROLL_ANYWHERE)
        except (AttributeError, NotImplementedError):
            pass
        self.focus(prefer_native="toolkit:gtk" in node.getAttributes())
        time.sleep(.1)
        rect = component.getExtents(pyatspi.DESKTOP_COORDS)
        assert rect.width > 0 and rect.height > 0, f"Not visible: {node.name}"
        point = (rect.x + rect.width // 2, rect.y + rect.height // 2)
        windows = [n.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                   for n in self.app() if n and n.getRoleName() in ["frame", "dialog", "file chooser"]]
        assert any(r.x <= point[0] < r.x + r.width and r.y <= point[1] < r.y + r.height for r in windows), f"Control outside the application window: {node.name}"
        from Xlib.ext import xtest
        xtest.fake_input(self.x, X.MotionNotify, x=point[0], y=point[1])
        xtest.fake_input(self.x, X.ButtonPress, 1)
        xtest.fake_input(self.x, X.ButtonRelease, 1)
        self.x.sync()
        time.sleep(.15)

    def fill(self, element_id, value):
        node = self.by_id(element_id)
        for _ in range(3):
            self.click(node)
            node.clear_cache()
            if node.getState().contains(pyatspi.STATE_FOCUSED) or node.getRoleName() == "combo box":
                break
            time.sleep(.25)
        assert node.getState().contains(pyatspi.STATE_FOCUSED) or node.getRoleName() == "combo box", f"Keyboard target is not focused: {element_id}"
        self.type_text(value)
        if node.getRoleName() != "password text":
            def committed():
                current = self.find(element_id)
                if current is None:
                    return False
                current.clear_cache()
                return current.queryText().getText(0, -1).replace("\r\n", "\n").rstrip("\n") == value.replace("\r\n", "\n").rstrip("\n")
            self.wait(committed, f"completed keyboard input in {element_id}")

    def keyboard_group(self):
        state = XkbState()
        assert self.keyboard.XkbGetState(self.keyboard_display, 0x100, ctypes.byref(state)) == 0
        return state.group

    def character_key(self, symbol):
        group = self.keyboard_group()
        cached = self.keymap.get((group, symbol))
        if cached:
            return cached
        # Xlib's core keymap indexes can represent another layout, not AltGr.
        for level in range(4):
            for code in range(8, 256):
                if self.keyboard.XkbKeycodeToKeysym(self.keyboard_display, code, group, level) == symbol:
                    self.keymap[(group, symbol)] = (code, level)
                    return code, level
        raise AssertionError(f"Character not available in the active keyboard layout: {chr(symbol)!r}")

    def type_text(self, value):
        self.key("a", control=True)
        from Xlib.ext import xtest
        shift = self.x.keysym_to_keycode(XK.string_to_keysym("Shift_L"))
        for char in value:
            symbol = ord(char)
            code, level = self.character_key(symbol)
            if level >= 2:
                try:
                    altgr = self.character_key(0xFE03)[0]
                except AssertionError:
                    altgr = self.character_key(0xFF7E)[0]
                xtest.fake_input(self.x, X.KeyPress, altgr)
            if level % 2:
                xtest.fake_input(self.x, X.KeyPress, shift)
            xtest.fake_input(self.x, X.KeyPress, code)
            xtest.fake_input(self.x, X.KeyRelease, code)
            if level % 2:
                xtest.fake_input(self.x, X.KeyRelease, shift)
            if level >= 2:
                xtest.fake_input(self.x, X.KeyRelease, altgr)
            self.x.flush()
            time.sleep(.005)
        self.x.sync()
        time.sleep(.15)

    def upload_attachment(self):
        attachment = self.root / "desktop-attachment.bin"
        attachment.write_bytes(b"desktop attachment\x00\xff")
        self.click("noteFile")
        self.wait(lambda: any(n.getRoleName() == "file chooser" for n in self.walk()), "native file chooser")
        self.key("l", control=True)
        self.type_text(str(attachment))
        self.key("Return")
        self.wait(lambda: attachment.name in self.text(self.app()), "attachment filename")
        self.saved()
        self.check("attachment selected through the native GTK file chooser")

    def enabled(self, element_id):
        node = self.find(element_id)
        if node is None:
            return False
        node.clear_cache()
        return node.getState().contains(pyatspi.STATE_ENABLED)

    def pick_transfer(self, path, directory=False, chooser_title=None):
        self.click("pickImportFolderBtn" if directory else "pickImportFilesBtn")
        self.wait(lambda: any(n.getRoleName() == "file chooser" for n in self.walk()), "native import chooser")
        if chooser_title:
            self.wait(lambda: any(chooser_title in n.name for n in self.walk() if n.getRoleName() in ["file chooser", "dialog"]), "translated native chooser title")
            self.screenshot("kab-09-native-import", prefer_native=True)
        self.key("l", control=True); self.type_text(str(path))
        if directory:
            # Accept the typed directory rather than navigating into it and
            # accidentally accepting its first child directory.
            choose = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "push button" and n.name in ["Select", "Open", "Choose"]), None), "directory selection")
            choose.queryAction().doAction(0)
        else:
            self.key("Return")
        self.wait(lambda: self.enabled("commitImportBtn" if directory else "prepareImportBtn"), "selected import files")

    def close_transfer(self):
        self.wait(lambda: self.enabled("closeTransferBtn"), "transfer operation completed")
        self.click("closeTransferBtn")

    def downloaded(self, pattern):
        return self.wait(lambda: next((p for p in (self.root / "Downloads").glob(pattern) if p.stat().st_size > 0), None), f"download {pattern}", timeout=30)

    def download_attachment(self):
        self.click(self.button("📎 Télécharger : desktop-attachment.bin"))
        downloaded = self.root / "Downloads/desktop-attachment.bin"
        self.wait(lambda: downloaded.exists(), "WebKit attachment download")
        assert downloaded.read_bytes() == b"desktop attachment\x00\xff"
        self.check("attachment download preserves exact binary bytes")

    def key(self, name, control=False, shift=False):
        from Xlib.ext import xtest
        code = self.character_key(ord(name))[0] if len(name) == 1 else self.x.keysym_to_keycode(XK.string_to_keysym(name))
        ctrl = self.x.keysym_to_keycode(XK.string_to_keysym("Control_L"))
        shift_code = self.x.keysym_to_keycode(XK.string_to_keysym("Shift_L"))
        if control:
            xtest.fake_input(self.x, X.KeyPress, ctrl)
        if shift:
            xtest.fake_input(self.x, X.KeyPress, shift_code)
        xtest.fake_input(self.x, X.KeyPress, code)
        xtest.fake_input(self.x, X.KeyRelease, code)
        if shift:
            xtest.fake_input(self.x, X.KeyRelease, shift_code)
        if control:
            xtest.fake_input(self.x, X.KeyRelease, ctrl)
        self.x.sync()
        time.sleep(.1)

    def select(self, element_id, index):
        self.click(element_id)
        # WebKitGTK's native popup does not consistently handle Home.
        for _ in range(8):
            self.key("Up")
        for _ in range(index):
            self.key("Down")
        self.key("Return")

    def button(self, name):
        return self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "push button" and n.name == name), None), name)

    def check(self, name):
        self.checks.append(name)
        print(f"PASS: {name}", flush=True)

    def screenshot(self, name, prefer_native=False):
        import gi
        gi.require_version("Gtk", "3.0")
        gi.require_version("GdkX11", "3.0")
        from gi.repository import Gtk, Gdk, GdkX11
        Gtk.init([])
        window = GdkX11.X11Window.foreign_new_for_display(Gdk.Display.get_default(), self.window(prefer_native).id)
        _, _, width, height = window.get_geometry()
        image = Gdk.pixbuf_get_from_window(window, 0, 0, width, height)
        assert image is not None
        image.savev(str(self.root / f"{name}.png"), "png", [], [])

    def launch(self):
        self.log = (self.root / "app.log").open("a")
        self.process = subprocess.Popen([str(self.binary)], cwd=self.root, env=self.env,
                                        stdout=self.log, stderr=self.log, start_new_session=True)
        self.wait(lambda: self.find("newVaultName"), "initial WebKit document", timeout=60)
        self.check("real WebKit startup and Tauri bridge")

    def stop(self):
        if self.process and self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()
        if self.log:
            self.log.close()
        if self.keyboard_display:
            if self.keyboard_group() != self.initial_group:
                self.keyboard.XkbLockGroup(self.keyboard_display, 0x100, self.initial_group)
            self.keyboard.XCloseDisplay(self.keyboard_display)
            self.keyboard_display = None

    def unlock(self):
        self.fill("unlockPass", self.vault_password)
        self.click("unlockBtn")
        self.by_id("newNoteBtn")
        profile = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "section" and self.text(n).startswith("👤 Validation")), None), "profile")
        self.click(profile)
        self.expect_text("currentProfileTitle", "Validation")

    def card(self, title):
        return self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "heading" and n.name == title), None), f"note card {title}")

    def password_prompt(self, password):
        self.fill("sysInput", password)
        self.click("sysOk")

    def saved(self):
        self.wait(lambda: "✓" in self.text("saveStatus"), "automatic save", timeout=35)

    def close_native(self):
        window = self.window()
        event = protocol.event.ClientMessage(window=window, client_type=self.x.intern_atom("WM_PROTOCOLS"), data=(32, [self.x.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0]))
        window.send_event(event, event_mask=X.NoEventMask)
        self.x.flush()
        self.wait(lambda: self.process.poll() is not None, "native close flush", timeout=25)
        self.log.close()

    def capture_ready(self):
        return self.wait(lambda: (node := self.find("captureContent")) and node.getState().contains(pyatspi.STATE_ENABLED), "quick capture ready")

    def palette_query(self, query):
        self.key("k", control=True)
        self.fill("commandQuery", query)
        self.wait(lambda: self.by_id("commandQuery").queryText().getText(0, -1) == query, "exact palette keyboard input")
        self.wait(lambda: "Recherche dans" not in self.text("commandStatus"), "palette search response")

    def run_commands(self):
        self.launch()
        self.fill("newVaultName", "DesktopCommands")
        self.fill("newVaultPass", self.vault_password)
        self.click("createVaultBtn")
        self.by_id("commandsBtn")
        self.key("n", control=True, shift=True)
        self.fill("captureProfileName", "Validation")
        self.click("createCaptureProfileBtn")
        self.capture_ready()
        self.fill("captureContent", "desktop quick idea")
        self.expect_text("captureStatus", "✓")
        self.screenshot("05-quick-capture")
        self.key("Return", control=True)
        self.card("desktop quick idea")
        self.check("quick capture shortcut, inline profile creation, encrypted draft and keyboard publication")
        self.palette_query("desktop quick")
        self.expect_text("commandResults", "desktop quick idea")
        self.screenshot("06-command-palette")
        self.key("Return")
        self.expect_text("vContent", "desktop quick idea")
        self.click("closeViewBtn")
        self.check("Ctrl+K searches and opens a note with Enter")
        self.palette_query(">capture")
        self.key("Return")
        self.capture_ready()
        self.fill("captureTitle", "desktop preserved draft")
        self.fill("captureContent", "desktop ongoing draft")
        self.key("Escape")
        self.expect_text("draftCount", "1")
        self.palette_query(">brouillon")
        self.key("Return")
        self.click(self.button("Reprendre"))
        self.expect_text("noteContent", "desktop ongoing draft")
        self.key("n", control=True, shift=True)
        self.capture_ready()
        self.fill("captureContent", "desktop parallel idea")
        self.key("Return", control=True)
        self.wait(lambda: self.find("captureContent") is None, "capture closed")
        self.expect_text("noteContent", "desktop ongoing draft")
        self.click("cancelNoteBtn")
        self.check("palette actions and independent capture preserve an ongoing full-editor draft")
        self.key("n", control=True, shift=True)
        self.capture_ready()
        self.fill("captureContent", "desktop transfer idea")
        self.click("captureFullBtn")
        self.expect_text("noteContent", "desktop transfer idea")
        self.click("saveNoteBtn")
        self.card("desktop transfer idea")
        self.check("quick draft transfers to the full editor without duplicate notes")
        self.fill("profileName", "Travail")
        self.click("addProfileBtn")
        self.key("n", control=True, shift=True)
        self.capture_ready()
        self.select("captureProfile", 1)
        self.fill("captureTitle", "desktop work note")
        self.fill("captureContent", "desktop work body")
        self.key("Return", control=True)
        self.expect_text("currentProfileTitle", "Travail")
        self.click(self.card("desktop work note"))
        self.click("editViewBtn")
        self.fill("tagInput", "focus")
        self.key("Return")
        self.click("saveNoteBtn")
        self.palette_query("@Validation")
        self.key("Return")
        self.expect_text("currentProfileTitle", "Validation")
        self.palette_query("#focus")
        self.key("Return")
        self.expect_text("currentProfileTitle", "#focus")
        self.card("desktop work note")
        self.check("profile and tag scopes navigate across the entire vault")
        self.click(self.card("desktop work note"))
        self.click("editViewBtn")
        self.select("noteProtection", 1)
        self.fill("notePass", self.note_password)
        self.saved()
        self.click("cancelNoteBtn")
        self.palette_query("desktop work note")
        self.expect_text("commandStatus", "Aucun résultat")
        self.key("Escape")
        self.key("k", control=True)
        protected = self.wait(lambda: next((n for n in self.walk()
            if any(a.startswith("id:command-result-") for a in n.getAttributes()) and "Note verrouillée" in self.text(n)), None), "masked protected note in recent results")
        self.click(protected)
        self.by_id("sysInput")
        self.key("Escape")
        self.wait(lambda: self.find("sysInput") is None, "keyboard cancellation of password dialog")
        self.check("protected notes stay masked and password prompts can be cancelled with Escape")
        self.click("lockBtn")
        self.by_id("unlockPass")
        self.key("n", control=True, shift=True)
        self.expect_text("vaultMsg", "Déverrouillez")
        assert self.find("captureContent") is None
        self.fill("unlockPass", self.vault_password)
        self.click("unlockBtn")
        self.capture_ready()
        assert self.by_id("captureContent").queryText().getText(0, -1) == ""
        self.key("Escape")
        self.check("locked-vault capture requests unlock and resumes with empty private fields")
        self.key("n", control=True, shift=True)
        self.capture_ready()
        self.fill("captureContent", "desktop close quick")
        self.close_native()
        self.launch()
        self.unlock()
        self.click("draftsBtn")
        self.click(self.button("Reprendre"))
        self.expect_text("noteContent", "desktop close quick")
        self.check("native close flushes an in-progress quick capture for recovery after restart")

    def highlight_word(self, word):
        from Xlib.ext import xtest
        node = self.by_id("vContent")
        node.queryComponent().scrollTo(pyatspi.SCROLL_ANYWHERE)
        text = node.queryText()
        content = text.getText(0, -1)
        offset = content.index(word)
        first = text.getCharacterExtents(offset, pyatspi.DESKTOP_COORDS)
        last = text.getCharacterExtents(offset + len(word) - 1, pyatspi.DESKTOP_COORDS)
        def rect(value):
            return (value.x, value.y, value.width, value.height) if hasattr(value, 'x') else tuple(value)
        x, y, _, h = rect(first)
        end_x, _, end_w, _ = rect(last)
        self.focus()
        xtest.fake_input(self.x, X.MotionNotify, x=x + 1, y=y + h // 2)
        xtest.fake_input(self.x, X.ButtonPress, 1)
        for i in range(1, 8):
            xtest.fake_input(self.x, X.MotionNotify, x=round(x + (end_x + end_w - x) * i / 7), y=y + h // 2)
            self.x.flush(); time.sleep(.02)
        xtest.fake_input(self.x, X.ButtonRelease, 1); self.x.sync()
        yellow = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "push button" and n.name in ["●", "Jaune"]), None), "highlight toolbar")
        self.click(yellow)
        time.sleep(.5)
        assert self.find("sysOk") is None, "Selection must remain valid after a stable link"
        self.screenshot("09-stable-link-highlight")

    def run_organization(self):
        self.launch()
        self.fill("newVaultName", "DesktopOrganization")
        self.fill("newVaultPass", self.vault_password)
        self.click("createVaultBtn")
        self.by_id("profileName")
        self.fill("profileName", "Validation"); self.click("addProfileBtn")
        profile = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "section" and self.text(n).startswith("👤 Validation")), None), "created profile")
        self.click(profile)
        self.click("newNoteBtn"); self.fill("noteTitle", "desktop target"); self.fill("noteContent", "desktop target preview"); self.click("saveNoteBtn")
        self.card("desktop target")
        self.click("newNoteBtn"); self.fill("noteTitle", "desktop source"); self.fill("noteContent", "Prefix ")
        self.click("pickNoteLinkBtn"); self.fill("commandQuery", "desktop target")
        self.expect_text("commandResults", "desktop target"); self.key("Return")
        field = self.by_id("noteContent"); field.clear_cache()
        raw = field.queryText().getText(0, -1)
        assert "|desktop target]]" in raw
        raw += " MARKWORD [[desktop missing]]"
        self.fill("noteContent", raw); self.click("saveNoteBtn"); self.card("desktop source")
        self.check("editor picker inserts a stable ID reference with a readable alias")
        self.fill("q", '"MARKWORD" modifiee:7j'); self.click("searchBtn")
        self.expect_text("currentProfileTitle", "1 résultat")
        self.click("saveSearchBtn"); self.fill("folderName", "desktop focus"); self.click("saveFolderBtn")
        self.expect_text("currentProfileTitle", "desktop focus"); self.expect_text("currentProfileTitle", "1 note")
        vault = next((self.root / "Documents/SecureNotes").glob("*.snotevault"))
        assert b"desktop focus" not in vault.read_bytes() and b"MARKWORD" not in vault.read_bytes()
        self.screenshot("07-smart-folder")
        self.check("advanced query is saved as a dynamic smart folder")
        self.click(self.card("desktop source")); self.expect_text("outgoingTitle", "(2)")
        self.highlight_word("MARKWORD")
        self.check("highlight selection remains valid after an aliased stable link")
        target_button = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "push button" and n.name.startswith("desktop target")), None), "outgoing target")
        self.click(target_button); self.expect_text("vTitle", "desktop target"); self.expect_text("incomingTitle", "(1)")
        self.click("editViewBtn"); self.fill("noteTitle", "desktop renamed target"); self.click("saveNoteBtn")
        self.click(self.card("desktop source")); self.expect_text("outgoingTitle", "(2)")
        self.wait(lambda: "desktop renamed target" in self.text(self.app()), "stable target after rename")
        self.screenshot("08-backlinks")
        self.check("incoming and outgoing links resolve the stable target after renaming")
        self.click(self.button("Créer cette note")); self.expect_text("noteTitle", "desktop missing")
        self.fill("noteContent", "created from reference"); self.click("saveNoteBtn")
        self.click(self.card("desktop source")); self.expect_text("outgoingTitle", "(2)")
        self.wait(lambda: not any(n.getRoleName() == "push button" and n.name == "Créer cette note" for n in self.walk()), "missing reference resolved")
        self.check("a missing title reference creates its target without changing the source text")
        self.click("editViewBtn"); self.fill("noteContent", raw.replace("MARKWORD", "done")); self.click("saveNoteBtn")
        self.expect_text("currentProfileTitle", "desktop focus"); self.expect_text("currentProfileTitle", "0 note")
        self.check("editing a note recomputes the active folder membership")
        self.click("folder-edit-0"); self.fill("folderName", "desktop done"); self.fill("folderQuery", '"done"'); self.click("saveFolderBtn")
        self.expect_text("currentProfileTitle", "desktop done"); self.expect_text("currentProfileTitle", "1 note")
        self.palette_query("/desktop done"); self.expect_text("commandResults", "desktop done"); self.key("Return")
        self.expect_text("currentProfileTitle", "desktop done")
        self.close_native(); self.launch(); self.unlock()
        self.click("folder-open-0"); self.expect_text("currentProfileTitle", "desktop done"); self.card("desktop source")
        self.screenshot("10-restored-folder")
        self.check("folder edits, palette navigation and saved criteria survive application restart")
        self.click("graphBtn"); self.click(self.button("Notes"))
        self.wait(lambda: "2 lien(s)" in self.text(self.app()), "stable and title links in graph")
        self.click("closeGraphBtn")
        self.click(self.button("Supprimer le dossier : desktop done")); self.click("sysOk")
        self.wait(lambda: self.find("folder-open-0") is None, "folder deleted")
        for title in ["desktop source", "desktop renamed target", "desktop missing"]: self.card(title)
        self.check("graph uses the same link resolution and deleting a folder preserves every note")

    def run_transfer(self):
        self.launch()
        self.fill("newVaultName", "DesktopTransferSource"); self.fill("newVaultPass", self.vault_password); self.click("createVaultBtn")
        self.by_id("profileName"); self.fill("profileName", "Validation"); self.click("addProfileBtn")
        profile = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "section" and self.text(n).startswith("👤 Validation")), None), "created profile")
        self.click(profile)
        folder = self.root / "markdown-import"; (folder / "images").mkdir(parents=True)
        (folder / "a.md").write_text('---\ntitle: Imported A\ntags: [from-folder]\n---\n# A\n[Vers B](b.md)\n![x](images/data.bin)', encoding="utf-8")
        (folder / "b.md").write_text("# Imported B\nbody", encoding="utf-8")
        (folder / "plain.txt").write_text("texte UTF-8 🌍", encoding="utf-8")
        (folder / "images/data.bin").write_bytes(b"folder resource\x00\xff")
        self.click("transferBtn"); self.click("transferImportTab"); self.pick_transfer(folder, directory=True)
        self.expect_text("transferPreviewSummary", "3 note(s)")
        self.click("transferPreserveProfiles"); self.click("commitImportBtn")
        self.expect_text("transferStatus", "3 note(s) ajoutée(s)"); self.close_transfer()
        self.click(self.card("Imported A")); self.expect_text("outgoingTitle", "(1)")
        self.click(self.button("📎 Télécharger : data.bin"))
        resource = self.downloaded("data.bin"); assert resource.read_bytes() == b"folder resource\x00\xff"
        self.click("closeViewBtn")
        self.check("native directory import handles Markdown/text, internal links and binary resources")
        self.click("newNoteBtn"); self.fill("noteTitle", "portable target"); self.fill("noteContent", "target body"); self.click("saveNoteBtn")
        self.card("portable target")
        self.click("newNoteBtn"); self.fill("noteTitle", "portable source")
        self.by_id("pickNoteLinkBtn").queryAction().doAction(0)
        self.fill("commandQuery", "portable target"); self.expect_text("commandResults", "portable target"); self.key("Return")
        field = self.by_id("noteContent"); field.clear_cache(); raw = field.queryText().getText(0, -1)
        self.fill("noteContent", raw + " portable body"); self.fill("tagInput", "travel"); self.key("Return")
        self.upload_attachment(); self.click("saveNoteBtn"); self.card("portable source")
        self.click("transferBtn"); self.select("transferFormat", 1)
        self.fill("transferFilter", "portable"); self.click("transferSelectNone"); self.click("transferSelectAll"); self.click("exportNotesBtn")
        markdown = self.downloaded("SecureNotes-Markdown-*.zip")
        self.wait(lambda: zipfile.is_zipfile(markdown), "complete Markdown ZIP")
        with zipfile.ZipFile(markdown) as archive:
            manifest = json.loads(archive.read("manifest.json")); assert len(manifest["notes"]) == 2
            exported = next(entry for entry in manifest["notes"] if entry["note"]["title"] == "portable source")
            assert exported["note"]["tags"] == ["travel"]
            assert archive.read(exported["attachment_file"]) == b"desktop attachment\x00\xff"
        self.select("transferFormat", 2); self.click("exportNotesBtn")
        text_zip = self.downloaded("SecureNotes-Texte-*.zip"); self.wait(lambda: zipfile.is_zipfile(text_zip), "complete text ZIP")
        with zipfile.ZipFile(text_zip) as archive:
            assert any(name.endswith(".txt") for name in archive.namelist())
        self.screenshot("11-export-selection"); self.close_transfer()
        self.check("selected Markdown/text exports download valid ZIPs with tags and exact attachment bytes")
        self.fill("profileName", "Destination"); self.click("addProfileBtn")
        self.click("transferBtn"); self.click("transferImportTab"); self.pick_transfer(markdown)
        self.click("prepareImportBtn"); self.expect_text("transferPreviewSummary", "2 identifiant(s) déjà présent(s)")
        self.click("transferPreserveProfiles"); self.select("transferDestination", 1); self.click("transferCopies")
        self.screenshot("12-import-preview"); self.click("commitImportBtn"); self.expect_text("transferStatus", "2 note(s) ajoutée(s)"); self.close_transfer()
        self.expect_text("currentProfileTitle", "Destination"); self.click(self.card("portable source")); self.expect_text("outgoingTitle", "(1)")
        target = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "push button" and n.name.startswith("portable target")), None), "copied link target")
        assert "Destination" in self.text(target)
        self.download_attachment(); self.click("closeViewBtn")
        self.check("copy import preserves originals, retargets internal links to copies and restores attachments")
        self.click("newNoteBtn"); self.fill("noteTitle", "portable private"); self.fill("noteContent", "secondary secret")
        self.select("noteProtection", 1); self.fill("notePass", self.note_password); self.click("saveNoteBtn"); self.card("🔒 Note verrouillée")
        archive_password = "portable-archive-password"
        self.click("transferBtn"); self.fill("transferPassword", archive_password); self.fill("transferConfirm", archive_password); self.click("exportNotesBtn")
        encrypted = self.downloaded("*.snotetransfer")
        self.wait(lambda: self.enabled("closeTransferBtn"), "encrypted export completed")
        envelope = json.loads(encrypted.read_text()); assert envelope["format"] == "securenotes-transfer"
        assert b"secondary secret" not in encrypted.read_bytes() and b"portable private" not in encrypted.read_bytes()
        self.close_transfer(); self.click("lockBtn"); self.by_id("unlockPass")
        self.fill("newVaultName", "DesktopTransferDestination"); self.fill("newVaultPass", self.vault_password); self.click("createVaultBtn"); self.by_id("transferBtn")
        self.palette_query(">import"); self.key("Return"); self.pick_transfer(encrypted)
        self.fill("transferImportPassword", "wrong-password"); self.click("prepareImportBtn"); self.expect_text("transferStatus", "incorrect")
        assert not self.enabled("commitImportBtn")
        self.fill("transferImportPassword", archive_password); self.click("prepareImportBtn"); self.expect_text("transferPreviewSummary", "1 protégée(s)")
        self.wait(lambda: self.enabled("commitImportBtn"), "encrypted import prepared")
        assert not self.enabled("transferCopies") and "portable private" not in self.text(self.app())
        self.click("commitImportBtn"); self.expect_text("transferStatus", "3 note(s) ajoutée(s)"); self.close_transfer()
        self.click(self.card("🔒 Note verrouillée")); self.password_prompt(archive_password); self.expect_text("vContent", "incorrect"); self.click("closeViewBtn")
        self.click(self.card("🔒 Note verrouillée")); self.password_prompt(self.note_password); self.expect_text("vContent", "secondary secret"); self.click("closeViewBtn")
        self.check("encrypted exchange between distinct vaults retains the independent note password")
        self.click("transferBtn"); self.click("transferImportTab"); self.pick_transfer(encrypted)
        self.fill("transferImportPassword", archive_password); self.click("prepareImportBtn"); self.expect_text("transferPreviewSummary", "3 identifiant(s) déjà présent(s)")
        self.click("commitImportBtn"); self.expect_text("transferStatus", "0 note(s) ajoutée(s), 3 identifiant(s) existant(s) conservé(s)")
        self.check("reimport is idempotent and never overwrites existing note identities")

    def controls_in_window(self, *ids):
        window = self.window().get_geometry()
        for element_id in ids:
            node = self.by_id(element_id)
            node.queryComponent().scrollTo(pyatspi.SCROLL_ANYWHERE)
            time.sleep(.1)
            rect = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
            # Use the AT-SPI frame coordinates; window-manager decorations vary.
            frames = [n.queryComponent().getExtents(pyatspi.DESKTOP_COORDS) for n in self.app() if n and n.getRoleName() == "frame"]
            assert rect.width > 0 and rect.height > 0, f"Empty control: {element_id}"
            assert any(r.x <= rect.x and r.y <= rect.y and rect.x + rect.width <= r.x + r.width and rect.y + rect.height <= r.y + r.height for r in frames), f"Clipped control: {element_id}: {rect} ({window.width}x{window.height})"

    def run_kabyle(self):
        self.launch()
        self.click("langKab")
        self.expect_text("unlockBtn", "Kkes awgelhen yerna ldi")
        self.expect_text("createVaultBtn", "Rnu asenduq awgelhan")
        self.fill("newVaultName", "DesktopKabyle"); self.fill("newVaultPass", "short"); self.click("createVaultBtn")
        self.expect_text("vaultMsg", "Awal uffir wezzil aṭas")
        self.screenshot("kab-01-vault")
        self.fill("newVaultPass", self.vault_password); self.click("createVaultBtn")
        self.by_id("profileName"); self.fill("profileName", "Validation"); self.click("addProfileBtn")
        profile = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "section" and self.text(n).startswith("👤 Validation")), None), "created profile")
        self.click(profile)
        self.controls_in_window("commandsBtn", "quickCaptureBtn", "graphBtn", "transferBtn", "dashBtn", "lockBtn")
        self.screenshot("kab-02-main")
        self.check("Kabyle vault creation, translated validation errors and main toolbar layout")

        self.click("newNoteBtn"); self.fill("noteTitle", "Kabyle validation note"); self.fill("noteContent", "Desktop Kabyle content and attachment")
        self.upload_attachment()
        self.expect_text("saveStatus", "Arewway")
        self.controls_in_window("noteProtection", "cancelNoteBtn", "saveNoteBtn")
        self.screenshot("kab-03-editor"); self.click("saveNoteBtn"); self.card("Kabyle validation note")
        self.click(self.card("Kabyle validation note"))
        self.expect_text("incomingTitle", "Iseɣwan i d-yettawin")
        self.expect_text("outgoingTitle", "Iseɣwan seg")
        self.controls_in_window("closeViewBtn", "editViewBtn", "stableLinkBtn", "delViewBtn")
        self.screenshot("kab-04-reader")
        self.button("📎 Sider: desktop-attachment.bin").queryAction().doAction(0)
        assert self.downloaded("desktop-attachment.bin").read_bytes() == b"desktop attachment\x00\xff"
        self.click("stableLinkBtn"); self.expect_text("sysMsg", "Asulu-is ad yeqqim d ameɣtu")
        self.controls_in_window("sysOk"); self.screenshot("kab-05-stable-dialog"); self.click("sysOk")
        self.click("delViewBtn"); self.expect_text("sysMsg", "Ad terreḍ tazmilt-a ɣer tqecwalt")
        self.controls_in_window("sysCancel", "sysOk"); self.click("sysCancel"); self.click("closeViewBtn")
        self.check("Kabyle editor, reader, long link headings, attachment download and confirmation dialogs")

        self.click("dashBtn")
        self.by_id("closeDashBtn").queryComponent().grabFocus(); self.key("Tab")
        self.wait(lambda: "Ass-a:" in self.text(self.app()), "Kabyle streak text")
        self.screenshot("kab-06-dashboard-streak")
        self.key("Tab"); self.key("Next")
        self.wait(lambda: "Awgelhen" in self.text(self.app()) and "war armud" in self.text(self.app()), "Kabyle security statistics")
        assert "Tafelwit n tedbelt" in self.text(self.app())
        self.screenshot("kab-06-dashboard"); self.click("closeDashBtn")
        self.click("graphBtn"); self.wait(lambda: "0 n yiseɣwan" in self.text(self.app()), "Kabyle graph counts")
        self.screenshot("kab-07-graph"); self.click("closeGraphBtn")
        self.key("n", control=True, shift=True); self.capture_ready(); self.fill("captureContent", "Kabyle quick capture")
        self.expect_text("captureStatus", "Arewway")
        self.screenshot("kab-08-capture"); self.key("Return", control=True); self.card("Kabyle quick capture")
        self.wait(lambda: self.find("captureContent") is None, "Kabyle capture closed after publication")
        self.by_id("commandsBtn").queryAction().doAction(0)
        self.fill("commandQuery", "Kabyle validation"); self.expect_text("commandResults", "Kabyle validation note"); self.key("Return")
        self.expect_text("vContent", "Desktop Kabyle content"); self.click("closeViewBtn")
        self.check("Kabyle dashboard, graph, quick capture and command palette navigation")

        self.fill("q", "Kabyle"); self.click("searchBtn"); self.expect_text("currentProfileTitle", "n yigmuḍen")
        self.click("saveSearchBtn"); self.fill("folderName", "Long saved search name for Kabyle layout validation")
        self.controls_in_window("saveFolderBtn", "cancelFolderBtn"); self.screenshot("kab-10-folder-editor")
        self.click("saveFolderBtn"); self.expect_text("currentProfileTitle", "Long saved search")
        self.check("Kabyle saved search form and translated dynamic result counts")

        self.click("newNoteBtn"); self.fill("noteTitle", "Kabyle private title"); self.fill("noteContent", "Kabyle private content")
        self.select("noteProtection", 1); self.fill("notePass", self.note_password); self.click("saveNoteBtn")
        self.click("clearSearchBtn"); self.card("🔒 Tazmilt isekkeṛen")
        self.click(self.card("🔒 Tazmilt isekkeṛen")); self.password_prompt("wrong-password")
        self.expect_text("vContent", "Awal uffir d arameɣtu")
        assert "Kabyle private content" not in self.text("vContent")
        self.click("closeViewBtn")
        self.click(self.card("🔒 Tazmilt isekkeṛen")); self.password_prompt(self.note_password)
        self.expect_text("vContent", "Kabyle private content"); self.click("relockViewBtn")
        self.check("Kabyle protected-note masking, translated password refusal and authenticated reading")

        self.click("transferBtn"); self.select("transferFormat", 1)
        self.expect_text("transferStatus", "ttwakksent seg tefrant war awgelhen")
        self.fill("transferFilter", "Kabyle validation"); self.click("transferSelectNone"); self.click("transferSelectAll")
        self.controls_in_window("exportNotesBtn"); self.screenshot("kab-11-export")
        self.click("exportNotesBtn"); markdown = self.downloaded("SecureNotes-Markdown-*.zip")
        self.wait(lambda: zipfile.is_zipfile(markdown), "complete Kabyle Markdown ZIP")
        with zipfile.ZipFile(markdown) as archive:
            manifest = json.loads(archive.read("manifest.json")); assert len(manifest["notes"]) == 1
            assert archive.read(manifest["notes"][0]["attachment_file"]) == b"desktop attachment\x00\xff"
        self.expect_text("transferStatus", "ttwaznent i usider"); self.close_transfer()
        self.click("transferBtn"); self.click("transferImportTab"); self.pick_transfer(markdown); self.click("prepareImportBtn")
        self.expect_text("transferPreviewSummary", "1 n yismawen n usulu yellan yakan")
        self.click("transferCopies"); self.controls_in_window("commitImportBtn"); self.screenshot("kab-12-import-preview")
        self.click("commitImportBtn"); self.expect_text("transferStatus", "1 n tezmilin ttwarnant"); self.close_transfer()
        self.check("Kabyle Markdown export and copy import retain exact attachment bytes")

        archive_password = "kabyle-archive-password"
        self.click("transferBtn"); self.fill("transferPassword", archive_password); self.fill("transferConfirm", archive_password); self.click("exportNotesBtn")
        encrypted = self.downloaded("*.snotetransfer"); self.wait(lambda: self.enabled("closeTransferBtn"), "Kabyle encrypted export")
        assert b"Kabyle private content" not in encrypted.read_bytes()
        self.close_transfer(); self.click("transferBtn"); self.click("transferImportTab"); self.pick_transfer(encrypted)
        self.fill("transferImportPassword", "wrong-password"); self.click("prepareImportBtn"); self.expect_text("transferStatus", "Awal uffir d arameɣtu")
        assert not self.enabled("commitImportBtn")
        self.fill("transferImportPassword", archive_password); self.click("prepareImportBtn")
        self.expect_text("transferPreviewSummary", "1 yettummestnen")
        assert "Kabyle private title" not in self.text("transferPreview") and not self.enabled("transferCopies")
        self.click("commitImportBtn"); self.expect_text("transferStatus", "0 n tezmilin ttwarnant"); self.close_transfer()
        self.check("Kabyle encrypted exchange preview, password errors and duplicate handling")

        folder = self.root / "kabyle-import"; folder.mkdir()
        (folder / "note.md").write_text("# Kabyle folder import\nUTF-8: Taqbaylit ɣ ɛ ḍ ṭ", encoding="utf-8")
        self.click("transferBtn"); self.click("transferImportTab")
        self.pick_transfer(folder, directory=True, chooser_title="SecureNotes — Kter akaram")
        self.expect_text("transferPreviewSummary", "1 n tezmilin")
        self.click("transferPreserveProfiles"); self.click("commitImportBtn"); self.expect_text("transferStatus", "1 n tezmilin ttwarnant"); self.close_transfer()
        self.click(self.card("Kabyle folder import")); self.expect_text("vContent", "Taqbaylit ɣ ɛ ḍ ṭ"); self.click("closeViewBtn")
        self.check("Kabyle native folder picker title and UTF-8 Markdown import")
        self.close_native(); self.launch(); self.expect_text("unlockBtn", "Kkes awgelhen yerna ldi")
        self.unlock(); self.expect_text("transferBtn", "Kter / Sifeḍ")
        self.screenshot("kab-13-restart")
        self.check("Kabyle language preference persists across native close and restart")

    def run_icons(self):
        self.launch()
        prop = self.wait(lambda: self.window().get_full_property(self.x.intern_atom("_NET_WM_ICON"), X.AnyPropertyType), "native window icon")
        values = list(prop.value)
        width, height = values[:2]
        assert width == height and width >= 32, f"Unexpected window icon size: {width}x{height}"
        import gi
        gi.require_version("GdkPixbuf", "2.0")
        from gi.repository import GdkPixbuf
        source = Path(__file__).resolve().parents[1] / "src-tauri/icons"
        image = GdkPixbuf.Pixbuf.new_from_file(str(source / (f"{width}x{height}.png" if width in [32, 64, 128] else "icon.png")))
        assert (image.get_width(), image.get_height()) == (width, height)
        pixels, stride, channels = image.get_pixels(), image.get_rowstride(), image.get_n_channels()
        expected = []
        for y in range(height):
            for x in range(width):
                offset = y * stride + x * channels
                r, g, b = pixels[offset:offset + 3]
                alpha = pixels[offset + 3] if channels == 4 else 255
                expected.append((alpha << 24) | (r << 16) | (g << 8) | b)
        assert values[2:2 + width * height] == expected, "Native window icon differs from the original SecureNotes artwork"
        self.check(f"native X11 window icon matches the generated SecureNotes PNG ({width}x{height})")
        self.fill("newVaultName", "DesktopIconValidation"); self.fill("newVaultPass", self.vault_password); self.click("createVaultBtn")
        self.by_id("newNoteBtn"); self.screenshot("icon-main-window")
        self.check("rebuilt application opens a vault with the original logo in its main header")

    def run(self, quick=False):
        self.launch()
        self.click("langKab")
        self.wait(lambda: self.by_id("unlockBtn").name != "Déchiffrer & Ouvrir ✨", "Kabyle translation")
        self.click("langFr")
        self.screenshot("01-vault")
        self.check("French / Kabyle language switch")
        self.fill("newVaultName", "DesktopValidation")
        self.fill("newVaultPass", self.vault_password)
        self.click("createVaultBtn")
        self.by_id("profileName")
        vaults = list((self.root / "Documents/SecureNotes").glob("*.snotevault"))
        assert len(vaults) == 1, "Vault must be inside the isolated Documents directory"
        self.fill("profileName", "Validation")
        self.click("addProfileBtn")
        profile = self.wait(lambda: next((n for n in self.walk() if n.getRoleName() == "section" and self.text(n).startswith("👤 Validation")), None), "created profile")
        self.click(profile)
        self.expect_text("currentProfileTitle", "Validation")
        self.check("vault and profile creation through the actual UI")
        self.click("newNoteBtn")
        assert self.by_id("noteContent").queryComponent().getExtents(pyatspi.DESKTOP_COORDS).height >= 150, "The writing area must not collapse inside the scrollable modal"
        self.fill("noteTitle", "desktop test note")
        self.fill("noteContent", "desktop secret content one")
        self.upload_attachment()
        self.saved()
        self.expect_text("saveStatus", "Brouillon")
        self.click("cancelNoteBtn")
        self.expect_text("draftCount", "1")
        self.click("draftsBtn")
        self.click(self.button("Reprendre"))
        self.expect_text("noteContent", "desktop secret content one")
        self.click("saveNoteBtn")
        self.card("desktop test note")
        self.check("new-note autosave, draft recovery and explicit publication")
        self.click(self.card("desktop test note"))
        self.click("editViewBtn")
        self.fill("noteContent", "desktop secret content two")
        self.saved()
        self.expect_text("saveStatus", "Note enregistrée")
        self.click("cancelNoteBtn")
        self.click(self.card("desktop test note"))
        self.expect_text("vContent", "desktop secret content two")
        self.download_attachment()
        self.check("existing-note autosave persisted after editor close")
        self.click("editViewBtn")
        self.select("noteProtection", 1)
        self.fill("notePass", self.note_password)
        self.saved()
        self.screenshot("02-protected-editor")
        self.click("cancelNoteBtn")
        self.card("🔒 Note verrouillée")
        self.click(self.card("🔒 Note verrouillée"))
        self.password_prompt("wrong-password")
        self.expect_text("vContent", "incorrect")
        assert "desktop secret content" not in self.text("vContent")
        self.click("closeViewBtn")
        self.click(self.card("🔒 Note verrouillée"))
        self.password_prompt(self.note_password)
        self.expect_text("vContent", "desktop secret content two")
        self.check("individual encryption, wrong-password refusal and authenticated reading")
        self.click("editViewBtn")
        self.password_prompt(self.note_password)
        self.by_id("noteContent")
        self.fill("noteContent", "desktop secret content three")
        self.select("noteProtection", 1)
        self.fill("notePass", self.new_password)
        self.saved()
        self.click("cancelNoteBtn")
        self.click(self.card("🔒 Note verrouillée"))
        self.password_prompt(self.note_password)
        self.expect_text("vContent", "incorrect")
        self.click("closeViewBtn")
        self.click(self.card("🔒 Note verrouillée"))
        self.password_prompt(self.new_password)
        self.expect_text("vContent", "desktop secret content three")
        self.button("📎 Télécharger : desktop-attachment.bin")
        self.click("relockViewBtn")
        self.wait(lambda: self.find("vContent") is None, "individual relock")
        self.check("protected editing, password rotation and individual relock")
        self.click(self.card("🔒 Note verrouillée"))
        self.password_prompt(self.new_password)
        self.click("editViewBtn")
        self.password_prompt(self.new_password)
        self.select("noteProtection", 2)
        self.saved()
        self.click("cancelNoteBtn")
        self.click(self.card("desktop test note"))
        self.expect_text("vContent", "desktop secret content three")
        self.click("closeViewBtn")
        self.check("authenticated removal of individual protection")
        self.click("newNoteBtn")
        self.fill("noteTitle", "protected draft test")
        self.fill("noteContent", "desktop protected draft")
        self.select("noteProtection", 1)
        self.fill("notePass", self.new_password)
        self.saved()
        self.click("cancelNoteBtn")
        self.click("draftsBtn")
        self.click(self.button("Reprendre"))
        self.by_id("sysInput")
        self.screenshot("04-protected-draft-prompt")
        self.password_prompt(self.new_password)
        self.expect_text("noteContent", "desktop protected draft")
        self.click("saveNoteBtn")
        self.check("protected draft recovery through its real password dialog")
        self.click("newNoteBtn")
        self.fill("noteTitle", "recovery after native close")
        self.fill("noteContent", "desktop recoverable draft")
        window = self.window()
        event = protocol.event.ClientMessage(window=window, client_type=self.x.intern_atom("WM_PROTOCOLS"), data=(32, [self.x.intern_atom("WM_DELETE_WINDOW"), X.CurrentTime, 0, 0, 0]))
        window.send_event(event, event_mask=X.NoEventMask)
        self.x.flush()
        self.wait(lambda: self.process.poll() is not None, "native close flush", timeout=25)
        self.log.close()
        self.launch()
        self.unlock()
        self.click("draftsBtn")
        self.click(self.button("Reprendre"))
        self.expect_text("noteContent", "desktop recoverable draft")
        self.click("saveNoteBtn")
        self.card("recovery after native close")
        self.check("native window close, restart and encrypted draft recovery")
        self.screenshot("03-main")
        self.click("lockBtn")
        self.by_id("unlockPass")
        self.unlock()
        self.check("manual vault lock and unlock")
        if not quick:
            self.select("autoLock", 0)
            self.click("dashBtn")
            self.wait(lambda: "1 min d'inactivité" in self.text(self.app()), "backend one-minute timeout displayed in dashboard")
            self.click("closeDashBtn")
            self.click("newNoteBtn")
            self.fill("noteContent", "desktop inactivity draft")
            self.saved()
            self.wait(lambda: self.find("unlockPass"), "one-minute inactivity lock", timeout=75)
            self.unlock()
            self.expect_text("draftCount", "1")
            self.check("real one-minute idle expiration preserves the saved draft")
        raw = vaults[0].read_bytes()
        for secret in [b"desktop secret content", b"desktop recoverable draft", self.note_password.encode(), self.new_password.encode()]:
            assert secret not in raw, "Plaintext or password found in vault file"
        self.check("encrypted on-disk storage in an isolated profile")

    def report(self, error=None):
        tree = []
        try:
            for n in self.walk():
                tree.append({"role": n.getRoleName(), "name": n.name, "attributes": n.getAttributes()})
        except Exception:
            pass
        (self.root / "accessibility-tree.json").write_text(json.dumps(tree, ensure_ascii=False, indent=2))
        report = {"binary": str(self.binary), "sandbox": str(self.root), "checks": self.checks, "error": str(error) if error else None}
        (self.root / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))
        print(json.dumps(report, ensure_ascii=False, indent=2), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default=str(Path(__file__).resolve().parents[1] / "src-tauri/target/release/secure-notes-tauri"))
    parser.add_argument("--quick", action="store_true", help="Skip the real one-minute inactivity wait")
    parser.add_argument("--commands", action="store_true", help="Validate command palette and quick capture workflows")
    parser.add_argument("--organization", action="store_true", help="Validate smart folders and stable links/backlinks")
    parser.add_argument("--transfer", action="store_true", help="Validate directory import, Markdown/text ZIPs and encrypted exchange")
    parser.add_argument("--kabyle", action="store_true", help="Validate Kabyle translations, dialogs, layout and import/export")
    parser.add_argument("--icons", action="store_true", help="Validate the native window icon against the original artwork and launch the app")
    args = parser.parse_args()
    smoke = DesktopSmoke(args.binary)
    print(f"Isolated test profile: {smoke.root}", flush=True)
    error = None
    try:
        if args.icons:
            smoke.run_icons()
        elif args.kabyle:
            smoke.run_kabyle()
        elif args.transfer:
            smoke.run_transfer()
        elif args.organization:
            smoke.run_organization()
        elif args.commands:
            smoke.run_commands()
        else:
            smoke.run(quick=args.quick)
    except Exception as exc:
        error = exc
        try:
            smoke.screenshot("failure")
            smoke.screenshot("failure-native", prefer_native=True)
        except Exception:
            pass
        raise
    finally:
        smoke.report(error)
        smoke.stop()


if __name__ == "__main__":
    main()
