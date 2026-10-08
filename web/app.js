"use strict";
(() => {
  // ------------------------------------------------------------------ utils
  const $ = (selector, root = document) => root.querySelector(selector);
  const textEncoder = new TextEncoder();
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
  const isTouch = window.matchMedia("(pointer: coarse)").matches;

  /** Tiny DOM builder: element("div", {class: "x", onclick: fn}, child, "text") */
  function element(tag, attributes, ...children) {
    const node = document.createElement(tag);
    for (const [name, value] of Object.entries(attributes || {})) {
      if (value == null || value === false) continue;
      if (name.startsWith("on")) node.addEventListener(name.slice(2), value);
      else if (name === "class") node.className = value;
      else if (name === "style" && typeof value === "object")
        for (const [property, styleValue] of Object.entries(value)) property.startsWith("--") ? node.style.setProperty(property, styleValue) : (node.style[property] = styleValue);
      else if (name === "html") node.innerHTML = value; // only ever used with static SVG strings
      else node.setAttribute(name, value === true ? "" : value);
    }
    for (const child of children.flat()) if (child != null && child !== false) node.append(child.nodeType ? child : String(child));
    return node;
  }

  const ICON = {
    term: '<svg viewBox="0 0 24 24"><path d="M5 7l5 5-5 5M12 18h7"/></svg>',
    play: '<svg viewBox="0 0 24 24"><path d="M7 5l12 7-12 7z"/></svg>',
    more: '<svg viewBox="0 0 24 24"><circle cx="5" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="19" cy="12" r="1.2"/></svg>',
    close: '<svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18"/></svg>',
    trash: '<svg viewBox="0 0 24 24"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-12M9 7V4h6v3"/></svg>',
    edit: '<svg viewBox="0 0 24 24"><path d="M4 20h4L19 9l-4-4L4 16v4z"/></svg>',
    copy: '<svg viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/></svg>',
    ext: '<svg viewBox="0 0 24 24"><path d="M14 4h6v6M20 4l-9 9M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"/></svg>',
    chev: '<svg viewBox="0 0 24 24" class="twisty"><path d="M9 6l6 6-6 6"/></svg>',
    folder: '<svg viewBox="0 0 24 24" class="ico"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
    file: '<svg viewBox="0 0 24 24" class="ico"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/></svg>',
    lock: '<svg viewBox="0 0 24 24" class="lock"><rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/></svg>',
    root: '<svg viewBox="0 0 24 24"><path d="M3 12l9-8 9 8M5 10v10h14V10"/></svg>',
    insert: '<svg viewBox="0 0 24 24"><path d="M4 12h12M12 6l6 6-6 6"/></svg>',
    plus: '<svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>',
    download: '<svg viewBox="0 0 24 24"><path d="M12 4v11M7 10l5 5 5-5M5 19h14"/></svg>',
    upload: '<svg viewBox="0 0 24 24"><path d="M12 19V8M7 13l5-5 5 5M5 4h14"/></svg>',
    newfile: '<svg viewBox="0 0 24 24"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M12 11v6M9 14h6"/></svg>',
    newfolder: '<svg viewBox="0 0 24 24"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M12 10v6M9 13h6"/></svg>',
    code: '<svg viewBox="0 0 24 24"><path d="M9 8l-4 4 4 4M15 8l4 4-4 4"/></svg>',
  };
  const icon = (name) => element("span", { html: ICON[name], style: { display: "contents" } });

  const COLORS = ["#4493f8", "#3fb950", "#d29922", "#f85149", "#a371f7", "#db61a2", "#39c5cf", "#8b949e"];

  function toast(msg, kind = "") {
    const el = element("div", { class: `toast ${kind}` }, msg);
    $("#toasts").append(el);
    setTimeout(() => el.remove(), kind === "error" ? 6000 : 2800);
  }

  async function api(method, path, body) {
    let response;
    try {
      response = await fetch(path, {
        method,
        headers: body !== undefined ? { "content-type": "application/json" } : {},
        body: body !== undefined ? JSON.stringify(body) : undefined,
        credentials: "same-origin",
        redirect: "manual",
      });
    } catch (e) {
      throw new Error("Connection to the server lost");
    }
    if (response.type === "opaqueredirect") {
      throw new Error("Cloudflare Access session expired — reload the page");
    }
    const data = await response.json().catch(() => ({}));
    if (!response.ok) {
      const error = new Error(data.error || (response.status === 403 ? "Access denied. Check your Cloudflare Access session." : `${response.status} ${response.statusText}`));
      error.status = response.status;
      throw error;
    }
    return data;
  }

  function shellQuote(text) {
    // Drop control characters first (ESC, newline, ^U, DEL…). A file name is
    // attacker-controlled: left in, an ESC[201~ would end the terminal's
    // bracketed paste early and the rest would run as typed commands. Then
    // single-quote unless the text is plainly safe.
    text = stripControl(text);
    return /^[\w@%+=:,./~-]+$/.test(text) ? text : `'${text.replace(/'/g, `'\\''`)}'`;
  }
  /** Remove C0/C1 control characters and DEL. */
  function stripControl(text) {
    // eslint-disable-next-line no-control-regex
    return text.replace(/[\u0000-\u001f\u007f-\u009f]/g, "");
  }
  function basename(path) {
    const parts = path.split("/").filter(Boolean);
    return parts.length ? parts[parts.length - 1] : "/";
  }
  function parentOf(path) {
    const separator = path.replace(/\/+$/, "").lastIndexOf("/");
    return separator <= 0 ? "/" : path.slice(0, separator);
  }
  function joinPath(dir, name) {
    return dir === "/" ? `/${name}` : `${dir}/${name}`;
  }
  function isUnder(path, root) {
    return path === root || path.startsWith(root === "/" ? "/" : root + "/");
  }
  function prettyPath(path) {
    const home = state.me?.home;
    if (home && isUnder(path, home)) return "~" + path.slice(home.length);
    return path;
  }
  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
      toast("Copied to clipboard", "ok");
    } catch {
      toast("Cannot access the clipboard", "error");
    }
  }

  // ------------------------------------------------------------ persistence
  const store = {
    key: (key) => `codeenv:${state.me?.email || "_"}:${key}`,
    get(key, fallback) {
      try {
        const value = localStorage.getItem(this.key(key));
        return value == null ? fallback : JSON.parse(value);
      } catch {
        return fallback;
      }
    },
    set(key, value) {
      try {
        localStorage.setItem(this.key(key), JSON.stringify(value));
      } catch {}
    },
  };

  // ------------------------------------------------------------------ state
  const state = {
    me: null,
    terms: [], // from server
    views: new Map(), // id -> TerminalView
    order: [], // open tab ids
    active: null,
    ports: [],
    // explorer
    viewRoot: "",
    selected: "", // selected path in tree
    selectedIsDir: true,
    expanded: new Set(),
    listings: new Map(), // dir path -> {entries, error, loading}
    showHidden: false,
    fontSize: 13,
  };

  /** Folder used for "new terminal" and buttons without a cwd. */
  function currentDir() {
    if (!state.selected) return state.viewRoot;
    return state.selectedIsDir ? state.selected : parentOf(state.selected);
  }

  // ------------------------------------------------------------- terminals
  const XTERM_THEME = {
    background: "#0d1117",
    foreground: "#d1d7e0",
    cursor: "#4493f8",
    cursorAccent: "#0d1117",
    selectionBackground: "rgba(56,139,253,0.35)",
    black: "#484f58", red: "#ff7b72", green: "#3fb950", yellow: "#d29922",
    blue: "#58a6ff", magenta: "#bc8cff", cyan: "#39c5cf", white: "#b1bac4",
    brightBlack: "#6e7681", brightRed: "#ffa198", brightGreen: "#56d364", brightYellow: "#e3b341",
    brightBlue: "#79c0ff", brightMagenta: "#d2a8ff", brightCyan: "#56d4dd", brightWhite: "#f0f6fc",
  };

  class TerminalView {
    constructor(id) {
      this.kind = "term";
      this.id = id;
      this.ws = null;
      this.closed = false;
      this.retry = 0;
      this.retryTimer = null;
      this.el = element("div", { class: "view", "data-id": id });
      $("#views").append(this.el);

      this.term = new Terminal({
        fontFamily: getComputedStyle(document.documentElement).getPropertyValue("--font-term"),
        fontSize: state.fontSize,
        lineHeight: 1.15,
        cursorBlink: true,
        scrollback: 50000,
        allowProposedApi: true,
        macOptionIsMeta: true,
        // When a program tracks the mouse, Option+drag (macOS) / Shift+drag
        // still selects text in the browser.
        macOptionClickForcesSelection: true,
        theme: XTERM_THEME,
      });
      this.fit = new FitAddon.FitAddon();
      this.term.loadAddon(this.fit);
      this.term.loadAddon(new WebLinksAddon.WebLinksAddon((ev, uri) => openLink(uri)));
      this.term.loadAddon(new Unicode11Addon.Unicode11Addon());
      this.term.unicode.activeVersion = "11";
      this.term.open(this.el);
      // WebGL pays off on large desktop terminals. Touch devices keep xterm's
      // DOM renderer: plenty for a phone screen, and mobile browsers cap the
      // number of live WebGL contexts (one per tab here).
      if (!isTouch) {
        try {
          const renderer = new WebglAddon.WebglAddon();
          renderer.onContextLoss(() => renderer.dispose());
          this.term.loadAddon(renderer);
        } catch {
          /* DOM renderer fallback */
        }
      }
      // Every key goes to the program; the only exception is ⌘K on macOS,
      // which a terminal never receives anyway.
      this.term.attachCustomKeyEventHandler((e) => !isAppShortcut(e));
      // Only the active view of a visible page sends text input and replies
      // to terminal queries (cursor position, device attributes…).
      this.term.onData((data) => this.isPrimary() && this.send(applyStickyCtrl(data)));
      this.term.onBinary((data) => this.send(Uint8Array.from(data, (c) => c.charCodeAt(0))));
      this.term.onResize(({ cols, rows }) => this.sendControl({ type: "resize", cols, rows }));
      // fit() is a no-op until the renderer has measured its cells; refit on
      // the first real render so the PTY never stays at xterm's 80×24.
      const firstRender = this.term.onRender(() => {
        firstRender.dispose();
        this.fitNow();
      });
      this.term.onSelectionChange(() => {
        const selection = this.term.getSelection();
        if (selection) navigator.clipboard?.writeText(selection).catch(() => {});
      });

      // Drop files/folders from the explorer: insert their path.
      acceptFileDrops(this.el, () => this.info()?.cwd || null);
      this.el.addEventListener("dragover", (e) => {
        if (e.dataTransfer.types.includes("text/x-codeenv-path")) {
          e.preventDefault();
          this.el.classList.add("drop-target");
        }
      });
      this.el.addEventListener("dragleave", () => this.el.classList.remove("drop-target"));
      this.el.addEventListener("drop", (e) => {
        this.el.classList.remove("drop-target");
        const path = e.dataTransfer.getData("text/x-codeenv-path");
        if (path) {
          e.preventDefault();
          this.paste(shellQuote(path) + " ");
        }
      });

      this.tab = makeTab(this, {
        title: this.info()?.title || "terminal",
        cls: "connecting",
        closeTitle: "Close terminal (right-click to hide the tab and keep it running)",
        onDblClick: (title) => startRename(this.id, title),
        onContextMenu: (e) => showTermMenu(e.clientX, e.clientY, this.id),
      });
      this.connect();
    }

    focus() {
      this.term.focus();
    }

    info() {
      return state.terms.find((terminal) => terminal.id === this.id);
    }

    setTitle(title) {
      if (this.titleEl.textContent !== title) this.titleEl.textContent = title;
      this.tab.title = title;
    }

    setState(status) {
      this.tab.classList.toggle("connecting", status === "connecting");
      this.tab.classList.toggle("dead", status === "dead");
    }

    connect() {
      clearTimeout(this.retryTimer);
      if (this.closed) return;
      if (this.ws) {
        const previous = this.ws;
        this.ws = null;
        previous.close();
      }
      this.setState("connecting");
      const protocol = location.protocol === "https:" ? "wss" : "ws";
      this.fitNow();
      const { cols, rows } = this.term;
      const ws = new WebSocket(`${protocol}://${location.host}/api/terminals/${this.id}/ws?cols=${cols}&rows=${rows}`);
      ws.binaryType = "arraybuffer";
      this.ws = ws;
      let ended = null;
      ws.onopen = () => {
        if (this.ws !== ws || this.closed) return;
        this.retry = 0;
        this.setState("ok");
        this.hideOverlay();
        // ptyd sends a snapshot: scrollback, screen, cursor and input modes.
        this.term.reset();
        // Resizes made while connecting were dropped: always announce the
        // current size, even if fit() finds nothing to change.
        this.fitNow();
        this.sendSize();
      };
      ws.onmessage = (ev) => {
        if (this.ws !== ws || this.closed) return;
        if (typeof ev.data === "string") {
          try {
            ended = JSON.parse(ev.data).type;
          } catch {}
          return;
        }
        this.term.write(new Uint8Array(ev.data));
        if (state.active !== this.id || document.hidden) this.tab.classList.add("activity");
      };
      ws.onclose = () => {
        if (this.ws !== ws || this.closed) return;
        this.ws = null;
        if (ended === "exit") {
          this.setState("dead");
          refreshTerms();
          setTimeout(() => removeTab(this.id), 400);
          return;
        }
        // Network drop or codeenv restart: the program keeps running in ptyd.
        const delay = Math.min(10000, 500 * 2 ** this.retry++);
        this.setState("connecting");
        this.showOverlay("Reconnecting…", true);
        this.retryTimer = setTimeout(() => this.connect(), delay);
      };
    }

    showOverlay(text, spinning) {
      this.hideOverlay();
      this.overlay = element(
        "div",
        { class: "overlay" },
        spinning ? element("span", { class: "spinner" }) : null,
        element("span", {}, text),
        element("button", { class: "ghost-btn", onclick: () => this.connect() }, spinning ? "Retry now" : "Reconnect"),
        this.retry > 3 ? element("button", { class: "ghost-btn", onclick: () => location.reload() }, "Reload page") : null,
      );
      this.el.append(this.overlay);
    }

    hideOverlay() {
      this.overlay?.remove();
      this.overlay = null;
    }

    isPrimary() {
      return state.active === this.id && document.visibilityState === "visible";
    }

    send(data) {
      if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(typeof data === "string" ? textEncoder.encode(data) : data);
    }

    sendControl(msg) {
      if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(msg));
    }

    paste(text) {
      // Defence in depth: never let a control char (ESC[201~, newline) reach
      // the PTY through a programmatic paste, whatever the caller passed.
      this.term.paste(stripControl(text));
      this.term.focus();
    }

    /** Views stay laid out (visibility, not display), so even background
     *  tabs can be measured and keep their PTY at the right size. */
    fitNow() {
      try {
        this.fit.fit();
      } catch {}
    }

    /** A font that finished loading after xterm measured its cells leaves
     *  the grid sized for the fallback font: re-measure, then refit. */
    remeasure() {
      const fontFamily = this.term.options.fontFamily;
      this.term.options.fontFamily = fontFamily.endsWith(" ") ? fontFamily.trimEnd() : `${fontFamily} `;
      this.fitNow();
    }

    sendSize() {
      this.sendControl({ type: "resize", cols: this.term.cols, rows: this.term.rows });
    }

    setFontSize(px) {
      this.term.options.fontSize = px;
      this.fitNow();
    }

    dispose() {
      this.closed = true;
      clearTimeout(this.retryTimer);
      this.ws?.close();
      this.term.dispose();
      this.el.remove();
      this.tab.remove();
    }
  }

  // ---------------------------------------------------------------- editor
  let codeMirrorLoading = null;
  /** CodeMirror (~400 kB gzip) is only fetched when a file is first opened. */
  function loadCodeMirror() {
    if (window.CM) return Promise.resolve(window.CM);
    codeMirrorLoading ??= new Promise((resolve, reject) => {
      const src = document.querySelector('meta[name="codemirror-src"]')?.content || "/assets/vendor/codemirror.js";
      const el = element("script", { src });
      el.onload = () => resolve(window.CM);
      el.onerror = () => {
        codeMirrorLoading = null;
        el.remove();
        reject(new Error("Cannot load the editor"));
      };
      document.head.append(el);
    });
    return codeMirrorLoading;
  }

  /** Indent unit used by the file: a tab, or the most common step of
   *  leading spaces between consecutive indented lines (2 by default). */
  function detectIndent(text) {
    let tabs = 0;
    const steps = new Map();
    let previousIndent = 0;
    for (const line of text.split("\n", 2000)) {
      if (!line.trim()) continue;
      if (line[0] === "\t") {
        tabs++;
        continue;
      }
      const indent = line.length - line.trimStart().length;
      const step = Math.abs(indent - previousIndent);
      if (step >= 2 && step <= 8) steps.set(step, (steps.get(step) || 0) + 1);
      previousIndent = indent;
    }
    const spaces = [...steps.values()].reduce((a, b) => a + b, 0);
    if (tabs > spaces) return "\t";
    let indentWidth = 2, count = 0;
    for (const [step, frequency] of steps) if (frequency > count) [indentWidth, count] = [step, frequency];
    return " ".repeat(indentWidth);
  }

  class FileEditor {
    constructor(path) {
      this.kind = "edit";
      this.id = `edit:${path}`;
      this.path = path;
      this.version = null;
      this.dirty = false;
      this.saving = false;
      this.loadVersion = 0;
      this.el = element("div", { class: "view editor-view", "data-id": this.id });
      this.host = element("div", { class: "cm-host" }, element("div", { class: "editor-msg" }, "Loading…"));
      this.statusEl = element("div", { class: "editor-status" });
      this.el.append(this.host, this.statusEl);
      $("#views").append(this.el);
      this.tab = makeTab(this, {
        title: basename(path),
        cls: "editor-tab",
        tooltip: path,
        closeTitle: "Close file",
        onContextMenu: (e) =>
          showMenu(e.clientX, e.clientY, [
            { label: "Save", icon: "edit", hint: isMac ? "⌘S" : "Ctrl+S", run: () => this.save() },
            { label: "Reload from disk", icon: "code", run: () => this.reload() },
            { label: "Download", icon: "download", run: () => downloadPath(this.path) },
            { label: "Copy path", icon: "copy", run: () => copyText(this.path) },
            { label: "Terminal in this folder", icon: "term", run: () => newTerminal(parentOf(this.path)) },
            "-",
            { label: "Close", icon: "close", run: () => closeTab(this.id) },
          ]),
      });
      this.tab.querySelector(".t-state").innerHTML = ICON.code;
      this.load();
    }

    async load() {
      const loadVersion = ++this.loadVersion;
      let CM, file;
      try {
        [CM, file] = await Promise.all([
          loadCodeMirror(),
          api("GET", `/api/fs/file?path=${encodeURIComponent(this.path)}`),
        ]);
      } catch (e) {
        if (loadVersion !== this.loadVersion) return;
        this.host.replaceChildren(
          element(
            "div",
            { class: "editor-msg" },
            element("p", {}, e.message),
            e.status === 404 || e.status === 400
              ? null
              : element("button", { class: "ghost-btn", onclick: () => downloadPath(this.path) }, icon("download"), "Download file"),
          ),
        );
        return;
      }
      if (loadVersion !== this.loadVersion) return;
      this.CM = CM;
      this.version = file.version;
      this.readonly = file.readonly;
      const language = CM.languageFor(this.path, file.content.slice(0, 200).split("\n")[0]);
      this.lang = language?.name || "Plain text";
      this.indent = detectIndent(file.content);
      this.eol = file.content.includes("\r\n") ? "CRLF" : "LF";
      this.fontConf = new CM.Compartment();
      const doc = CM.EditorState.create({
        doc: file.content,
        extensions: [
          CM.basicSetup,
          CM.keymap.of([
            { key: "Mod-s", preventDefault: true, run: () => (this.save(), true) },
            CM.indentWithTab,
          ]),
          CM.EditorView.theme(
            {
              "&": { height: "100%", backgroundColor: "var(--bg-term)" },
              ".cm-gutters": { backgroundColor: "var(--bg-term)", borderRight: "1px solid var(--border)" },
              ".cm-activeLineGutter, .cm-activeLine": { backgroundColor: "rgba(110,118,129,0.1)" },
              ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.5" },
              ".cm-panels": { backgroundColor: "var(--surface-2)" },
            },
            { dark: true },
          ),
          // Listed after ours: CodeMirror gives precedence to the first theme.
          CM.oneDark,
          this.fontConf.of(this.fontTheme()),
          language ? language.extension : [],
          CM.indentUnit.of(this.indent),
          CM.EditorState.tabSize.of(this.indent === "\t" ? 4 : this.indent.length),
          // Keep the file's line endings when saving.
          this.eol === "CRLF" ? CM.EditorState.lineSeparator.of("\r\n") : [],
          CM.EditorState.readOnly.of(file.readonly),
          CM.EditorView.updateListener.of((update) => {
            if (update.docChanged) this.setDirty(true);
            if (update.docChanged || update.selectionSet) this.renderStatus();
          }),
        ],
      });
      this.host.replaceChildren();
      this.editor = new CM.EditorView({ state: doc, parent: this.host });
      this.renderStatus();
      if (this.pendingGoto) {
        this.goto(...this.pendingGoto);
        this.pendingGoto = null;
      }
      if (state.active === this.id && this.wantFocus) this.focus();
    }

    async reload() {
      if (this.saving) return;
      if (this.dirty && !confirm("Discard unsaved changes?")) return;
      this.editor?.destroy();
      this.editor = null;
      this.setDirty(false);
      this.host.replaceChildren(element("div", { class: "editor-msg" }, "Loading…"));
      await this.load();
    }

    fontTheme() {
      return this.CM.EditorView.theme({ "&": { fontSize: `${state.fontSize}px` } });
    }

    text() {
      // sliceDoc joins lines with the document's own line separator.
      return this.editor.state.sliceDoc();
    }

    async save(force = false) {
      if (!this.editor || this.saving) return;
      if (this.readonly) return toast("Read-only file", "error");
      this.saving = true;
      this.renderStatus("Saving…");
      const content = this.text();
      try {
        const result = await api("PUT", "/api/fs/file", { path: this.path, content, version: force ? undefined : this.version });
        this.version = result.version;
        if (this.editor && this.text() === content) this.setDirty(false);
        this.renderStatus("Saved");
      } catch (e) {
        this.saving = false;
        if (e.status === 409 && !force) {
          if (confirm(`“${basename(this.path)}” has changed on disk since you opened it.\n\nOverwrite it with your version?`))
            return await this.save(true);
          this.renderStatus("Not saved: changed on disk");
          return;
        }
        toast(e.message, "error");
        this.renderStatus("Save failed");
      } finally {
        this.saving = false;
      }
    }

    setDirty(dirty) {
      if (this.dirty === dirty) return;
      this.dirty = dirty;
      this.tab.classList.toggle("dirty", dirty);
    }

    renderStatus(msg) {
      if (!this.editor) return;
      const editorState = this.editor.state;
      const head = editorState.selection.main.head;
      const line = editorState.doc.lineAt(head);
      const indent = this.indent === "\t" ? "Tabs" : `Spaces: ${this.indent.length}`;
      // replaceChildren() would print null as text.
      const parts = [
        element("span", { class: "es-path", title: this.path }, prettyPath(this.path)),
        element("span", { class: "spacer" }),
        msg ? element("span", { class: "es-msg" }, msg) : null,
        this.readonly ? element("span", { class: "badge" }, "read-only") : null,
        element("span", {}, `Ln ${line.number}, Col ${head - line.from + 1}`),
        element("span", {}, indent),
        element("span", {}, this.eol),
        element("span", {}, this.lang),
        element("button", { class: "ghost-btn es-save", disabled: this.readonly || null, onclick: () => this.save() }, this.dirty ? "Save •" : "Save"),
      ];
      this.statusEl.replaceChildren(...parts.filter(Boolean));
      if (msg === "Saved") setTimeout(() => this.renderStatus(), 1500);
    }

    /** Puts the cursor on `line` (1-based), selecting `selection` = [from, to]
     *  (UTF-16 offsets in the line, as CodeMirror counts) when given. */
    goto(line, selection) {
      if (!this.editor) {
        this.pendingGoto = [line, selection];
        return;
      }
      const doc = this.editor.state.doc;
      const targetLine = doc.line(Math.min(Math.max(1, line), doc.lines));
      const from = selection ? Math.min(targetLine.from + selection[0], targetLine.to) : targetLine.from;
      const to = selection ? Math.min(targetLine.from + selection[1], targetLine.to) : targetLine.from;
      this.editor.dispatch({
        selection: { anchor: from, head: to },
        effects: this.CM.EditorView.scrollIntoView(from, { y: "center" }),
      });
      if (state.active === this.id) this.editor.focus();
    }

    /** The file was renamed or moved. */
    retarget(path) {
      retargetView(this, path);
      if (this.CM) this.lang = this.CM.languageFor(path)?.name || this.lang;
      this.renderStatus();
    }

    focus() {
      this.editor?.focus();
    }

    fitNow() {}

    setFontSize() {
      this.editor?.dispatch({ effects: this.fontConf.reconfigure(this.fontTheme()) });
    }

    dispose() {
      this.loadVersion++;
      this.editor?.destroy();
      this.editor = null;
      this.el.remove();
      this.tab.remove();
    }
  }

  const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico", "svg"]);
  const extOf = (p) => (basename(p).includes(".") ? basename(p).split(".").pop().toLowerCase() : "");
  const mediaKind = (p) => (IMAGE_EXT.has(extOf(p)) ? "image" : extOf(p) === "pdf" ? "pdf" : null);

  /** Images and PDFs, shown by the browser itself. */
  class MediaView {
    constructor(path) {
      this.kind = "media";
      this.id = `edit:${path}`;
      this.path = path;
      this.el = element("div", { class: "view media-view", "data-id": this.id });
      this.stage = element("div", { class: "media-stage" });
      this.statusEl = element("div", { class: "editor-status" });
      this.el.append(this.stage, this.statusEl);
      $("#views").append(this.el);
      this.tab = makeTab(this, {
        title: basename(path),
        cls: "editor-tab",
        tooltip: path,
        closeTitle: "Close",
        onContextMenu: (e) =>
          showMenu(e.clientX, e.clientY, [
            { label: "Open in new tab", icon: "ext", run: () => window.open(this.url(), "_blank", "noopener") },
            { label: "Download", icon: "download", run: () => downloadPath(this.path) },
            { label: "Copy path", icon: "copy", run: () => copyText(this.path) },
            "-",
            { label: "Close", icon: "close", run: () => closeTab(this.id) },
          ]),
      });
      this.tab.querySelector(".t-state").innerHTML = ICON.file;
      this.render();
    }

    url() {
      // The timestamp defeats caches when the file is replaced on disk.
      return `/api/fs/raw?path=${encodeURIComponent(this.path)}&t=${Date.now()}`;
    }

    render() {
      const kind = mediaKind(this.path);
      const status = (...parts) =>
        this.statusEl.replaceChildren(
          element("span", { class: "es-path", title: this.path }, prettyPath(this.path)),
          element("span", { class: "spacer" }),
          ...parts.filter(Boolean),
          element("button", { class: "ghost-btn es-save", onclick: () => window.open(this.url(), "_blank", "noopener") }, "New tab"),
          element("button", { class: "ghost-btn es-save", onclick: () => downloadPath(this.path) }, "Download"),
        );
      if (kind === "pdf") {
        this.stage.replaceChildren(element("iframe", { class: "media-pdf", src: this.url(), title: basename(this.path) }));
        status(element("span", {}, "PDF"));
        return;
      }
      const img = element("img", { class: "media-img fit", src: this.url(), alt: basename(this.path), draggable: "false" });
      img.onload = () => status(element("span", {}, `${img.naturalWidth} × ${img.naturalHeight}`), element("span", {}, extOf(this.path).toUpperCase()));
      img.onerror = () => this.stage.replaceChildren(element("div", { class: "editor-msg" }, element("p", {}, "Cannot display this image.")));
      // Click toggles between "fit to the view" and actual size.
      img.onclick = () => img.classList.toggle("fit");
      this.stage.replaceChildren(img);
      status();
    }

    retarget(path) {
      retargetView(this, path);
      this.render();
    }

    focus() {}
    fitNow() {}
    setFontSize() {}

    dispose() {
      this.el.remove();
      this.tab.remove();
    }
  }

  function retargetView(view, path) {
    const old = view.id;
    view.path = path;
    view.id = `edit:${path}`;
    view.el.dataset.id = view.id;
    view.tab.dataset.id = view.id;
    view.titleEl.textContent = basename(path);
    view.tab.title = path;
    state.views.delete(old);
    state.views.set(view.id, view);
    state.order = state.order.map((x) => (x === old ? view.id : x));
    if (state.active === old) state.active = view.id;
  }

  function openEditor(path, { show = true, focus = true, line = 0, sel = null } = {}) {
    const id = `edit:${path}`;
    let view = state.views.get(id);
    if (!view) {
      view = mediaKind(path) ? new MediaView(path) : new FileEditor(path);
      state.views.set(id, view);
      if (!state.order.includes(id)) state.order.push(id);
      renderTabs();
    }
    view.wantFocus = focus;
    if (show) activate(id, { focus });
    if (line) view.goto?.(line, sel);
    saveTabs();
  }

  window.addEventListener("beforeunload", (e) => {
    if ([...state.views.values()].some((view) => view.dirty)) e.preventDefault();
  });

  /** Tab element shared by terminals and editors. */
  function makeTab(view, { title, cls = "", closeTitle, tooltip, onDblClick, onContextMenu }) {
    const titleEl = element("span", { class: "t-title" }, title);
    const tab = element(
      "div",
      {
        class: `tab ${cls}`,
        role: "tab",
        draggable: "true",
        title: tooltip || title,
        "data-id": view.id,
        // Clicking the active tab must not steal focus back (it would end
        // a rename started by double-click).
        onclick: () => state.active !== view.id && activate(view.id),
        onauxclick: (e) => e.button === 1 && closeTab(view.id),
        ondblclick: () => onDblClick?.(titleEl),
        oncontextmenu: (e) => {
          e.preventDefault();
          onContextMenu?.(e);
        },
        ondragstart: (e) => {
          e.dataTransfer.setData("text/x-codeenv-tab", view.id);
          e.dataTransfer.effectAllowed = "move";
        },
        ondragover: (e) => {
          if (e.dataTransfer.types.includes("text/x-codeenv-tab")) {
            e.preventDefault();
            tab.classList.add("drag-over");
          }
        },
        ondragleave: () => tab.classList.remove("drag-over"),
        ondrop: (e) => {
          tab.classList.remove("drag-over");
          const from = e.dataTransfer.getData("text/x-codeenv-tab");
          if (from && from !== view.id) {
            e.preventDefault();
            moveTab(from, view.id);
          }
        },
      },
      element("span", { class: "t-state" }),
      titleEl,
      element("button", {
        class: "t-close",
        title: closeTitle,
        html: ICON.close,
        onclick: (e) => {
          e.stopPropagation();
          closeTab(view.id);
        },
      }),
    );
    view.titleEl = titleEl;
    return tab;
  }

  function openLink(uri) {
    // A localhost link printed in the terminal (Vite, Next…) opens through
    // port forwarding — but only when forwarding is configured. Never rewrite
    // it onto the UI's own origin (there is no path mode).
    try {
      const parsedUrl = new URL(uri);
      const local = ["localhost", "127.0.0.1", "0.0.0.0", "[::1]", "[::]"].includes(parsedUrl.hostname);
      if (local && parsedUrl.port) {
        const url = portUrl(+parsedUrl.port, parsedUrl.pathname + parsedUrl.search + parsedUrl.hash);
        if (!url) {
          toast("Port forwarding is not configured", "error");
          return;
        }
        window.open(url, "_blank", "noopener");
        return;
      }
    } catch {}
    window.open(uri, "_blank", "noopener");
  }

  function openTab(id, { focus = true } = {}) {
    if (!state.views.has(id)) {
      const view = new TerminalView(id);
      state.views.set(id, view);
      if (!state.order.includes(id)) state.order.push(id);
      renderTabs();
    }
    if (focus) activate(id);
    saveTabs();
  }

  /** `focus: false` shows the tab but leaves the keyboard where it is
   *  (e.g. in the explorer after a single click on a file). */
  function activate(id, { focus = true } = {}) {
    const activeView = state.views.get(id);
    if (!activeView) return;
    state.active = id;
    for (const [vid, view] of state.views) {
      const on = vid === id;
      view.el.classList.toggle("active", on);
      view.tab.classList.toggle("active", on);
      view.tab.setAttribute("aria-selected", on);
    }
    activeView.tab.classList.remove("activity");
    activeView.tab.scrollIntoView({ block: "nearest", inline: "nearest" });
    requestAnimationFrame(() => {
      activeView.fitNow();
      if (focus) activeView.focus();
    });
    $("#empty").hidden = true;
    if (isTouch) $("#mobile-keys").hidden = activeView.kind !== "term";
    renderTermList();
    saveTabs();
    if (window.matchMedia("(max-width: 760px)").matches) document.body.classList.remove("sidebar-open");
  }

  /** Closing a terminal's tab ends the terminal (asking first if a program
   *  other than the shell is running). `{ keep: true }` only hides the tab. */
  async function closeTab(id, { keep = false } = {}) {
    const view = state.views.get(id);
    if (!view) return;
    if (view.dirty && !confirm(`“${basename(view.path)}” has unsaved changes. Close anyway?`)) return;
    if (view.kind === "term" && !keep && !view.closed) {
      // Fresh state: the periodic refresh may predate the command just typed.
      await refreshTerms();
      const terminal = view.info();
      if (terminal && terminal.command && !SHELLS.has(terminal.command) && !confirm(`“${terminal.command}” is still running in “${terminal.title}”. Close the terminal anyway?`))
        return;
      try {
        await api("DELETE", `/api/terminals/${id}`);
      } catch (e) {
        if (e.status !== 404) return toast(e.message, "error");
      }
      state.terms = state.terms.filter((x) => x.id !== id);
    }
    removeTab(id);
  }

  function removeTab(id) {
    const view = state.views.get(id);
    if (!view) return;
    const idx = state.order.indexOf(id);
    view.dispose();
    state.views.delete(id);
    state.order = state.order.filter((x) => x !== id);
    if (state.active === id) {
      state.active = null;
      const next = state.order[Math.min(idx, state.order.length - 1)];
      if (next) activate(next);
    }
    if (isTouch && !state.active) $("#mobile-keys").hidden = true;
    $("#empty").hidden = state.order.length > 0;
    renderTermList();
    saveTabs();
  }

  function moveTab(from, to) {
    const order = state.order.filter((x) => x !== from);
    order.splice(order.indexOf(to), 0, from);
    state.order = order;
    renderTabs();
    saveTabs();
  }

  function cycleTab(delta) {
    if (!state.order.length) return;
    const i = state.order.indexOf(state.active);
    activate(state.order[(i + delta + state.order.length) % state.order.length]);
  }

  function renderTabs() {
    const tabs = $("#tabs");
    tabs.replaceChildren(...state.order.map((id) => state.views.get(id).tab));
  }

  function saveTabs() {
    store.set("tabs", { order: state.order, active: state.active });
  }

  async function newTerminal(cwd = currentDir(), command = "", title = "") {
    try {
      const terminal = await api("POST", "/api/terminals", { cwd, command, title });
      state.terms.push(terminal);
      openTab(terminal.id);
      renderTermList();
    } catch (e) {
      toast(e.message, "error");
    }
  }

  async function killTerminal(id) {
    try {
      await api("DELETE", `/api/terminals/${id}`);
      removeTab(id);
      await refreshTerms();
    } catch (e) {
      toast(e.message, "error");
    }
  }

  function startRename(id, anchor) {
    const terminal = state.terms.find((x) => x.id === id);
    const input = element("input", { value: terminal?.title || "" });
    const original = anchor.textContent;
    anchor.replaceChildren(input);
    input.focus();
    input.select();
    let done = false;
    const finish = async (save) => {
      if (done) return;
      done = true;
      const title = input.value.trim();
      anchor.textContent = save && title ? title : original;
      if (save && title && title !== terminal?.title) {
        try {
          await api("PATCH", `/api/terminals/${id}`, { title });
          await refreshTerms();
        } catch (e) {
          toast(e.message, "error");
        }
      }
      state.views.get(id)?.focus();
    };
    input.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Enter") finish(true);
      if (e.key === "Escape") finish(false);
    });
    input.addEventListener("blur", () => finish(true));
    input.addEventListener("click", (e) => e.stopPropagation());
  }

  async function refreshTerms() {
    try {
      state.terms = await api("GET", "/api/terminals");
    } catch (e) {
      return;
    }
    for (const terminal of state.terms) state.views.get(terminal.id)?.setTitle(terminal.title);
    renderTermList();
  }

  const SHELLS = new Set(["bash", "zsh", "fish", "sh", "dash", "ksh", "login", "-bash", "-zsh"]);

  function renderTermList() {
    const ul = $("#term-list");
    $("#terms-count").textContent = state.terms.length || "";
    if (!state.terms.length) {
      ul.replaceChildren(element("li", { class: "empty" }, isTouch ? "No terminals. Tap + to open one." : "No terminals. Press Alt+Shift+T to open one."));
      return;
    }
    ul.replaceChildren(
      ...state.terms.map((terminal) => {
        const busy = terminal.command && !SHELLS.has(terminal.command);
        const title = element("span", { class: "title" }, terminal.title);
        const killBtn = element("button", {
          class: "icon-btn sm",
          title: "Terminate terminal",
          html: ICON.trash,
          onclick: (e) => {
            e.stopPropagation();
            confirmClick(killBtn, () => killTerminal(terminal.id));
          },
        });
        return element(
          "li",
          {
            class: state.active === terminal.id ? "active" : "",
            title: `${terminal.title}\n${terminal.cwd}\n${terminal.clients} connection(s)`,
            onclick: () => openTab(terminal.id),
            oncontextmenu: (e) => {
              e.preventDefault();
              showTermMenu(e.clientX, e.clientY, terminal.id);
            },
          },
          element("span", { class: `status-dot ${terminal.clients > 0 ? "attached" : ""}` }),
          element("span", { class: "main" }, title, element("span", { class: "sub" }, prettyPath(terminal.cwd))),
          busy ? element("span", { class: "badge busy" }, terminal.command) : null,
          element(
            "span",
            { class: "actions" },
            element("button", {
              class: "icon-btn sm",
              title: "Rename",
              html: ICON.edit,
              onclick: (e) => {
                e.stopPropagation();
                startRename(terminal.id, title);
              },
            }),
            killBtn,
          ),
        );
      }),
    );
  }

  /** Two-step confirmation on the same button, without a modal. */
  function confirmClick(btn, action) {
    if (btn.dataset.armed) {
      action();
      return;
    }
    btn.dataset.armed = "1";
    const title = btn.title;
    btn.style.color = "var(--red)";
    btn.title = "Click again to confirm";
    toast("Click again to confirm");
    setTimeout(() => {
      delete btn.dataset.armed;
      btn.style.color = "";
      btn.title = title;
    }, 3000);
  }

  function showTermMenu(x, y, id) {
    const terminal = state.terms.find((x) => x.id === id);
    const open = state.views.has(id);
    showMenu(x, y, [
      !open && { label: "Open", icon: "term", run: () => openTab(id) },
      {
        label: "Rename",
        icon: "edit",
        run: () => {
          if (!open) openTab(id);
          const view = state.views.get(id);
          startRename(id, view.titleEl);
        },
      },
      terminal && { label: "New terminal in this folder", icon: "plus", run: () => newTerminal(terminal.cwd) },
      terminal && { label: "Copy folder path", icon: "copy", run: () => copyText(terminal.cwd) },
      open && { label: "Hide tab (keep terminal running)", icon: "close", run: () => closeTab(id, { keep: true }) },
      "-",
      { label: "Close terminal", icon: "trash", danger: true, run: () => killTerminal(id) },
    ]);
  }

  // ------------------------------------------------------------- explorer
  async function loadDir(path, force = false) {
    const cached = state.listings.get(path);
    if (cached && !force && !cached.error) return cached;
    const entry = { entries: cached?.entries || [], loading: true, error: null };
    state.listings.set(path, entry);
    renderTree();
    try {
      const listing = await api("GET", `/api/fs?path=${encodeURIComponent(path)}`);
      entry.entries = listing.entries;
      entry.truncated = listing.truncated;
    } catch (e) {
      entry.error = e.message;
    }
    entry.loading = false;
    renderTree();
    return entry;
  }

  function visibleEntries(dir) {
    const listing = state.listings.get(dir);
    if (!listing) return [];
    return state.showHidden ? listing.entries : listing.entries.filter((e) => !e.name.startsWith("."));
  }

  /** Flat list of visible rows, in display order. */
  function visibleRows() {
    const rows = [];
    const walk = (dir, depth) => {
      for (const entry of visibleEntries(dir)) {
        const path = joinPath(dir, entry.name);
        rows.push({ path, entry: entry, depth });
        if (entry.dir && state.expanded.has(path)) walk(path, depth + 1);
      }
    };
    walk(state.viewRoot, 0);
    return rows;
  }

  function renderTree() {
    const tree = $("#tree");
    const fragment = document.createDocumentFragment();
    const walk = (dir, depth) => {
      const listing = state.listings.get(dir);
      if (!listing) return;
      if (listing.error) fragment.append(element("div", { class: "tree-msg", style: { paddingLeft: `${depth * 12 + 24}px` } }, listing.error));
      if (state.creating?.dir === dir) fragment.append(createRow(depth));
      for (const entry of visibleEntries(dir)) {
        const path = joinPath(dir, entry.name);
        const open = entry.dir && state.expanded.has(path);
        fragment.append(treeRow(path, entry, depth, open));
        if (open) walk(path, depth + 1);
      }
      if (listing.truncated) fragment.append(element("div", { class: "tree-msg", style: { paddingLeft: `${depth * 12 + 24}px` } }, "… listing truncated"));
      if (!listing.loading && !listing.error && !visibleEntries(dir).length)
        fragment.append(element("div", { class: "tree-msg", style: { paddingLeft: `${depth * 12 + 24}px` } }, "empty"));
    };
    walk(state.viewRoot, 0);
    tree.replaceChildren(fragment);
    const input = $("#tree .create-row input, #tree .rename-input");
    if (input) {
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    }
    const rootEl = $("#fs-root");
    // LRM marks keep "~/a/b" in order inside the rtl box used for start-ellipsis.
    rootEl.textContent = `\u200e${prettyPath(state.viewRoot)}\u200e`;
    rootEl.title = state.viewRoot;
    $("#fs-up").disabled = state.viewRoot === state.me.root;
  }

  function treeRow(path, entry, depth, open) {
    const loading = entry.dir && state.listings.get(path)?.loading;
    const row = element(
      "div",
      {
        class: [
          "node",
          entry.dir ? "dir" : "file",
          open && "open",
          entry.link && "link",
          entry.name.startsWith(".") && "hidden-file",
          state.selected === path && "selected",
          loading && "loading",
        ]
          .filter(Boolean)
          .join(" "),
        role: "treeitem",
        "aria-expanded": entry.dir ? String(open) : null,
        "data-path": path,
        draggable: "true",
        title: entry.dir
          ? `${prettyPath(path)}\nDouble-click: open terminal here · drop files here to upload`
          : `${prettyPath(path)} — ${formatSize(entry.size)}\nClick: open · double-click: edit · F2: rename`,
        style: { paddingLeft: `${depth * 12 + 4}px` },
        onclick: () => {
          select(path, entry.dir);
          if (entry.dir) toggleDir(path);
          else openEditor(path, { focus: false });
        },
        ondblclick: () => (entry.dir ? newTerminal(path) : openEditor(path)),
        oncontextmenu: (ev) => {
          ev.preventDefault();
          select(path, entry.dir);
          showFsMenu(ev.clientX, ev.clientY, path, entry.dir);
        },
        ondragstart: (ev) => {
          ev.dataTransfer.setData("text/x-codeenv-path", path);
          ev.dataTransfer.setData("text/plain", path);
          ev.dataTransfer.effectAllowed = "copyMove";
          state.dragging = path;
        },
        ondragend: () => (state.dragging = null),
        ondragover: (ev) => {
          const from = state.dragging;
          if (!entry.dir || !from || !ev.dataTransfer.types.includes("text/x-codeenv-path")) return;
          if (from === path || parentOf(from) === path || isUnder(path, from)) return;
          ev.preventDefault();
          ev.dataTransfer.dropEffect = "move";
          row.classList.add("drop-into");
        },
        ondragleave: () => row.classList.remove("drop-into"),
        ondrop: (ev) => {
          row.classList.remove("drop-into");
          const from = ev.dataTransfer.getData("text/x-codeenv-path");
          if (!entry.dir || !from || from === path || isUnder(path, from)) return;
          ev.preventDefault();
          ev.stopPropagation();
          if (confirm(`Move “${basename(from)}” to ${basename(path)}/?`)) moveEntry(from, joinPath(path, basename(from)));
        },
      },
      element("span", { html: ICON.chev, style: { display: "contents" } }),
      element("span", { html: entry.dir ? ICON.folder : ICON.file, style: { display: "contents" } }),
      state.renaming?.path === path ? renameInput(path, entry) : element("span", { class: "name" }, entry.name),
      element(
        "span",
        { class: "row-actions" },
        entry.dir
          ? element("button", {
              class: "icon-btn sm",
              title: "Open terminal here",
              html: ICON.term,
              onclick: (ev) => {
                ev.stopPropagation();
                newTerminal(path);
              },
            })
          : element("button", {
              class: "icon-btn sm",
              title: "Download",
              html: ICON.download,
              onclick: (ev) => {
                ev.stopPropagation();
                downloadPath(path);
              },
            }),
        element("button", {
          class: "icon-btn sm",
          title: "More actions",
          html: ICON.more,
          onclick: (ev) => {
            ev.stopPropagation();
            select(path, entry.dir);
            const bounds = ev.currentTarget.getBoundingClientRect();
            showFsMenu(bounds.left, bounds.bottom + 2, path, entry.dir);
          },
        }),
      ),
    );
    return row;
  }

  function renameInput(path, entry) {
    const renameState = state.renaming;
    const input = element("input", {
      class: "rename-input",
      value: renameState.value,
      spellcheck: "false",
      onclick: (ev) => ev.stopPropagation(),
      ondblclick: (ev) => ev.stopPropagation(),
      oninput: () => (renameState.value = input.value),
      onkeydown: async (ev) => {
        ev.stopPropagation();
        if (ev.key === "Escape") return cancel();
        if (ev.key !== "Enter") return;
        const name = input.value.trim();
        if (!name || name === entry.name) return cancel();
        await moveEntry(path, joinPath(parentOf(path), name));
        state.renaming = null;
        renderTree();
      },
      onblur: () => setTimeout(() => state.renaming === renameState && cancel(), 150),
    });
    const cancel = () => {
      state.renaming = null;
      renderTree();
      $("#tree").focus();
    };
    // Select the name without its extension, like file managers do.
    requestAnimationFrame(() => {
      const dot = entry.dir ? -1 : renameState.value.lastIndexOf(".");
      input.setSelectionRange(0, dot > 0 ? dot : renameState.value.length);
    });
    return input;
  }

  function startRenameEntry(path) {
    state.renaming = { path, value: basename(path) };
    renderTree();
  }

  /** Renames or moves; open editors follow, listings refresh. */
  async function moveEntry(from, to) {
    let result;
    try {
      result = await api("POST", "/api/fs/rename", { from, to });
    } catch (err) {
      toast(err.message, "error");
      return false;
    }
    const destination = result.path;
    for (const view of [...state.views.values()]) {
      if (view.path && isUnder(view.path, from)) view.retarget(destination + view.path.slice(from.length));
    }
    state.expanded = new Set([...state.expanded].map((p) => (isUnder(p, from) ? destination + p.slice(from.length) : p)));
    state.listings = new Map([...state.listings].map(([p, listing]) => [
      isUnder(p, from) ? destination + p.slice(from.length) : p, listing,
    ]));
    if (state.selected && isUnder(state.selected, from)) state.selected = destination + state.selected.slice(from.length);
    saveTree();
    saveTabs();
    await Promise.all([loadDir(parentOf(from), true), loadDir(parentOf(destination), true)]);
    return true;
  }

  async function deleteEntry(path, isDir) {
    const what = isDir ? `the folder “${basename(path)}” and all its contents` : `“${basename(path)}”`;
    if (!confirm(`Permanently delete ${what}?\n\n${prettyPath(path)}`)) return;
    try {
      await api("POST", "/api/fs/delete", { path });
    } catch (err) {
      return toast(err.message, "error");
    }
    for (const view of [...state.views.values()]) if (view.path && isUnder(view.path, path)) removeTab(view.id);
    state.expanded = new Set([...state.expanded].filter((p) => !isUnder(p, path)));
    for (const directory of [...state.listings.keys()]) if (isUnder(directory, path)) state.listings.delete(directory);
    if (state.selected && isUnder(state.selected, path)) state.selected = "";
    saveTree();
    await loadDir(parentOf(path), true);
    toast(`${basename(path)} deleted`, "ok");
  }

  /** Inline name input for a new file or folder in `dir`. */
  function startCreate(dir, isDir) {
    state.creating = { dir, isDir, value: "" };
    document.body.classList.remove("no-sidebar");
    $("#panel-files").classList.remove("collapsed");
    if (dir !== state.viewRoot && isUnder(dir, state.viewRoot)) state.expanded.add(dir);
    if (!isUnder(dir, state.viewRoot)) setViewRoot(dir);
    loadDir(dir).then(renderTree);
  }

  function createRow(depth) {
    const creation = state.creating;
    const input = element("input", {
      value: creation.value,
      placeholder: creation.isDir ? "folder name" : "file name",
      spellcheck: "false",
      oninput: () => (creation.value = input.value),
      onkeydown: async (e) => {
        e.stopPropagation();
        if (e.key === "Escape") return cancel();
        if (e.key !== "Enter" || !input.value.trim()) return;
        const path = joinPath(creation.dir, input.value.trim());
        try {
          await api("POST", "/api/fs/create", { path, dir: creation.isDir });
        } catch (err) {
          return toast(err.message, "error");
        }
        state.creating = null;
        await loadDir(creation.dir, true);
        select(path, creation.isDir);
        if (!creation.isDir) openEditor(path);
      },
      onblur: () => setTimeout(() => state.creating === creation && !input.value.trim() && cancel(), 150),
    });
    const cancel = () => {
      state.creating = null;
      renderTree();
    };
    return element(
      "div",
      { class: "node create-row", style: { paddingLeft: `${depth * 12 + 20}px` } },
      element("span", { html: creation.isDir ? ICON.folder : ICON.file, style: { display: "contents" } }),
      input,
    );
  }

  function select(path, isDir) {
    state.selected = path;
    state.selectedIsDir = isDir;
    for (const el of document.querySelectorAll("#tree .node.selected")) el.classList.remove("selected");
    document.querySelector(`#tree .node[data-path="${CSS.escape(path)}"]`)?.classList.add("selected");
  }

  async function toggleDir(path, force) {
    const open = force ?? !state.expanded.has(path);
    if (open) {
      state.expanded.add(path);
      await loadDir(path);
    } else {
      state.expanded.delete(path);
    }
    saveTree();
    renderTree();
  }

  function setViewRoot(path) {
    if (!isUnder(path, state.me.root)) path = state.me.root;
    state.viewRoot = path;
    state.selected = "";
    renderSearchScope();
    renderTree();
    saveTree();
    loadDir(path);
  }

  function saveTree() {
    store.set("tree", { viewRoot: state.viewRoot, expanded: [...state.expanded].slice(-200), hidden: state.showHidden });
  }

  async function refreshTree() {
    const dirs = [state.viewRoot, ...[...state.expanded].filter((p) => isUnder(p, state.viewRoot))];
    state.listings.clear();
    await Promise.all(dirs.map((d) => loadDir(d, true)));
  }

  function showFsMenu(x, y, path, isDir) {
    const dir = isDir ? path : parentOf(path);
    const buttons = allButtons();
    const active = state.views.get(state.active);
    const activeTerm = active?.kind === "term" ? active : null;
    showMenu(x, y, [
      !isDir && { label: mediaKind(path) ? "View" : "Open in editor", icon: "code", run: () => openEditor(path), hint: "click" },
      { label: "Open terminal here", icon: "term", run: () => newTerminal(dir), hint: isDir ? "double-click" : null },
      buttons.length && { section: `Run in ${basename(dir)}/` },
      ...buttons.map((b) => ({ label: b.name, color: b.color || COLORS[0], run: () => runButton(b, dir) })),
      "-",
      activeTerm && { label: "Insert path in terminal", icon: "insert", run: () => activeTerm.paste(shellQuote(path) + " ") },
      "-",
      { label: isDir ? "Download (zip)" : "Download", icon: "download", run: () => downloadPath(path) },
      { label: `Upload files to ${basename(dir)}/…`, icon: "upload", run: () => pickUpload(dir, false) },
      { label: `Upload folder to ${basename(dir)}/…`, icon: "upload", run: () => pickUpload(dir, true) },
      { label: "New file…", icon: "newfile", run: () => startCreate(dir, false) },
      { label: "New folder…", icon: "newfolder", run: () => startCreate(dir, true) },
      "-",
      { label: "Rename…", icon: "edit", hint: "F2", run: () => startRenameEntry(path) },
      { label: "Delete", icon: "trash", danger: true, hint: "Delete", run: () => deleteEntry(path, isDir) },
      "-",
      { label: "Copy path", icon: "copy", run: () => copyText(path) },
      isDir && { label: "Set as explorer root", icon: "root", run: () => setViewRoot(path) },
    ]);
  }

  function treeKeydown(e) {
    const rows = visibleRows();
    if (!rows.length) return;
    let i = rows.findIndex((r) => r.path === state.selected);
    const currentRow = rows[i];
    const go = (j) => {
      const r = rows[Math.max(0, Math.min(rows.length - 1, j))];
      select(r.path, r.entry.dir);
      document.querySelector(`#tree .node[data-path="${CSS.escape(r.path)}"]`)?.scrollIntoView({ block: "nearest" });
    };
    switch (e.key) {
      case "ArrowDown":
        go(i + 1);
        break;
      case "ArrowUp":
        go(i < 0 ? 0 : i - 1);
        break;
      case "Home":
        go(0);
        break;
      case "End":
        go(rows.length - 1);
        break;
      case "ArrowRight":
        if (currentRow?.entry.dir) {
          if (!state.expanded.has(currentRow.path)) toggleDir(currentRow.path, true);
          else go(i + 1);
        }
        break;
      case "ArrowLeft":
        if (currentRow?.entry.dir && state.expanded.has(currentRow.path)) toggleDir(currentRow.path, false);
        else if (currentRow) {
          const p = parentOf(currentRow.path);
          const j = rows.findIndex((r) => r.path === p);
          if (j >= 0) go(j);
        }
        break;
      case " ":
        if (currentRow?.entry.dir) toggleDir(currentRow.path);
        break;
      case "Enter":
        if (currentRow) currentRow.entry.dir ? newTerminal(currentRow.path) : openEditor(currentRow.path);
        break;
      case "F2":
        if (currentRow) startRenameEntry(currentRow.path);
        break;
      case "Delete":
      case "Backspace":
        if (currentRow && (e.key === "Delete" || e.metaKey)) deleteEntry(currentRow.path, currentRow.entry.dir);
        else return;
        break;
      case "ContextMenu":
        if (currentRow) {
          const r = document.querySelector(`#tree .node[data-path="${CSS.escape(currentRow.path)}"]`)?.getBoundingClientRect();
          if (r) showFsMenu(r.left + 24, r.bottom, currentRow.path, currentRow.entry.dir);
        }
        break;
      default:
        return;
    }
    e.preventDefault();
  }

  // --------------------------------------------------------------- buttons
  function allButtons() {
    return [...(state.me.global_buttons || []).map((button) => ({ ...button, global: true })), ...(state.me.buttons || [])];
  }

  function runButton(button, dir) {
    const cwd = button.cwd && dir === undefined ? button.cwd : dir ?? currentDir();
    newTerminal(cwd, button.command, button.name);
  }

  function renderButtons() {
    const wrap = $("#buttons");
    const buttons = allButtons();
    wrap.replaceChildren(
      ...buttons.map((button) =>
        element(
          "button",
          {
            class: "cmd-btn",
            style: { "--c": button.color || COLORS[0] },
            title: `${button.command}\n\nFolder: ${button.cwd || "selected folder"}\nShift+click: run in selected folder\nRight-click: options`,
            onclick: (e) => runButton(button, e.shiftKey ? currentDir() : undefined),
            oncontextmenu: (e) => {
              e.preventDefault();
              showMenu(e.clientX, e.clientY, [
                { label: "Run", icon: "play", run: () => runButton(button) },
                { label: `Run in ${basename(currentDir())}/`, icon: "play", run: () => runButton(button, currentDir()) },
                "-",
                !button.global && { label: "Edit", icon: "edit", run: () => editButton(button) },
                { label: "Duplicate", icon: "copy", run: () => editButton({ ...button, id: "", global: false, name: `${button.name} (copy)` }) },
                !button.global && { label: "Delete", icon: "trash", danger: true, run: () => deleteButton(button) },
              ]);
            },
          },
          element("span", { class: "dot" }),
          button.name,
          button.global ? icon("lock") : null,
        ),
      ),
      element("button", { class: "cmd-btn add", title: "Add command", onclick: () => editButton(null) }, "+ Command"),
    );
  }

  let editedButton = null;
  function editButton(button) {
    editedButton = button && button.id ? button : null;
    const dialog = $("#button-dialog");
    const form = $("#button-form");
    $("#button-dialog-title").textContent = editedButton ? "Edit command" : "New command";
    form.name.value = button?.name || "";
    form.command.value = button?.command || "";
    form.cwd.value = button?.cwd || "";
    const color = button?.color || COLORS[0];
    const colors = $(".colors", form);
    colors.replaceChildren(
      element("legend", {}, "Color"),
      ...COLORS.map((c) =>
        element("label", { title: c }, element("input", { type: "radio", name: "color", value: c, checked: c === color }), element("span", { style: { "--c": c } })),
      ),
    );
    $("#button-delete").hidden = !editedButton;
    dialog.showModal();
    form.name.focus();
  }

  async function saveButtons(list) {
    try {
      state.me.buttons = await api("PUT", "/api/buttons", list);
      renderButtons();
      return true;
    } catch (e) {
      toast(e.message, "error");
      return false;
    }
  }

  async function deleteButton(button) {
    if (await saveButtons(state.me.buttons.filter((x) => x.id !== button.id))) toast(`“${button.name}” deleted`, "ok");
  }

  function setupButtonDialog() {
    const dialog = $("#button-dialog");
    const form = $("#button-form");
    $("#button-cancel").onclick = () => dialog.close();
    $("#button-delete").onclick = async () => {
      if (editedButton) await deleteButton(editedButton);
      dialog.close();
    };
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      const button = {
        id: editedButton?.id || "",
        name: form.name.value.trim(),
        command: form.command.value.trim(),
        cwd: form.cwd.value.trim(),
        color: form.color.value,
      };
      const list = editedButton ? state.me.buttons.map((x) => (x.id === editedButton.id ? button : x)) : [...state.me.buttons, button];
      if (await saveButtons(list)) dialog.close();
    });
    // Keep shortcuts typed in the dialog away from the app.
    dialog.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) form.requestSubmit();
      e.stopPropagation();
    });
  }

  // ---------------------------------------------------------------- search
  const search = { ctl: null, case: false, regex: false, scope: null, files: [], collapsed: new Set(), timer: null };

  function searchDir() {
    return search.scope && isUnder(search.scope, state.me.root) ? search.scope : state.viewRoot;
  }

  function openSearch() {
    document.body.classList.remove("no-sidebar");
    if (isTouch) document.body.classList.add("sidebar-open");
    $("#panel-search").classList.remove("collapsed");
    renderSearchScope();
    const input = $("#search-input");
    input.focus();
    input.select();
  }

  function renderSearchScope() {
    const b = $("#search-scope");
    b.textContent = prettyPath(searchDir());
    b.title = `${searchDir()}\nClick: search in selected folder (${prettyPath(currentDir())})`;
  }

  function scheduleSearch(delay = 300) {
    clearTimeout(search.timer);
    search.timer = setTimeout(runSearch, delay);
  }

  async function runSearch() {
    clearTimeout(search.timer);
    search.ctl?.abort();
    const q = $("#search-input").value;
    search.files = [];
    search.collapsed.clear();
    $("#search-count").textContent = "";
    if (!q) {
      $("#search-summary").textContent = "";
      $("#search-count").textContent = "";
      renderSearchResults();
      return;
    }
    const controller = (search.ctl = new AbortController());
    $("#search-summary").textContent = "Searching…";
    renderSearchResults();
    const params = new URLSearchParams({ dir: searchDir(), q, regex: search.regex, case: search.case });
    try {
      const response = await fetch(`/api/search/text?${params}`, { signal: controller.signal, redirect: "manual" });
      if (controller.signal.aborted) return;
      if (response.type === "opaqueredirect") throw new Error("Session expired. Reload the page.");
      if (!response.ok) {
        const err = await response.json().catch(() => ({}));
        throw new Error(err.error || `Error ${response.status}`);
      }
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "", complete = false;
      try {
        for (;;) {
          const { value, done } = await reader.read();
          if (controller.signal.aborted) return;
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          let newline;
          while ((newline = buffer.indexOf("\n")) >= 0) {
            const line = buffer.slice(0, newline);
            buffer = buffer.slice(newline + 1);
            if (!line) continue;
            const msg = JSON.parse(line);
            if (msg.done) {
              complete = true;
              const plural = (count, word) => `${count} ${word}${count === 1 ? "" : "s"}`;
              $("#search-summary").textContent = `${plural(msg.matches, "result")} in ${plural(msg.files, "file")}${msg.truncated ? " (search limited)" : ""} · ${msg.ms} ms`;
              $("#search-count").textContent = msg.matches || "";
            } else {
              const i = search.files.findIndex((f) => f.file.localeCompare(msg.file) > 0);
              search.files.splice(i < 0 ? search.files.length : i, 0, msg);
              renderSearchResultsSoon();
            }
          }
        }
        if (!complete) throw new Error("Search interrupted. Try again.");
      } finally {
        await reader.cancel().catch(() => {});
        reader.releaseLock();
      }
    } catch (e) {
      if (!controller.signal.aborted) $("#search-summary").textContent = e.message;
    }
    if (!controller.signal.aborted) renderSearchResults();
  }

  let searchRenderTimer = null;
  function renderSearchResultsSoon() {
    searchRenderTimer ??= setTimeout(() => {
      searchRenderTimer = null;
      renderSearchResults();
    }, 80);
  }

  function highlighted(text, ranges) {
    const out = [];
    let at = 0;
    for (const [a, b] of ranges) {
      if (a < at) continue;
      out.push(text.slice(at, a), element("mark", {}, text.slice(a, b)));
      at = b;
    }
    out.push(text.slice(at));
    return out;
  }

  /** The sidebar is narrow: start the displayed line shortly before the
   *  first match so it is always visible (leading indentation dropped). */
  function visibleExcerpt(text, ranges) {
    const indent = text.length - text.trimStart().length;
    const first = ranges[0]?.[0] ?? 0;
    const start = first - indent > 24 ? first - 12 : indent;
    if (start <= 0) return highlighted(text, ranges);
    const shifted = ranges.map(([a, b]) => [a - start, b - start]).filter(([a]) => a >= 0);
    return [start > indent ? "…" : "", ...highlighted(text.slice(start), shifted)];
  }

  function renderSearchResults() {
    const base = searchDir();
    const items = [];
    for (const file of search.files) {
      const folded = search.collapsed.has(file.file);
      const rel = isUnder(file.file, base) ? file.file.slice(base.length).replace(/^\//, "") : file.file;
      const dir = rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : "";
      items.push(
        element(
          "li",
          {
            class: `sr-file${folded ? "" : " open"}`,
            title: file.file,
            onclick: () => {
              folded ? search.collapsed.delete(file.file) : search.collapsed.add(file.file);
              renderSearchResults();
            },
            oncontextmenu: (e) => {
              e.preventDefault();
              showFsMenu(e.clientX, e.clientY, file.file, false);
            },
          },
          element("span", { html: ICON.chev, style: { display: "contents" } }),
          element("span", { class: "sr-name" }, basename(file.file)),
          element("span", { class: "sr-dir" }, dir),
          element("span", { class: "badge" }, file.more ? `${file.matches.length}+` : file.matches.length),
        ),
      );
      if (folded) continue;
      for (const match of file.matches) {
        items.push(
          element(
            "li",
            {
              class: "sr-line",
              title: `${prettyPath(file.file)}:${match.line}`,
              onclick: () => {
                // Offsets are exact unless the line was cut around the match.
                const sel = match.text.startsWith("…") || !match.ranges.length ? null : match.ranges[0];
                openEditor(file.file, { line: match.line, sel });
                if (isTouch) document.body.classList.remove("sidebar-open");
              },
            },
            element("span", { class: "sr-ln" }, match.line),
            element("span", { class: "sr-text" }, ...visibleExcerpt(match.text, match.ranges)),
          ),
        );
      }
    }
    $("#search-results").replaceChildren(...items);
  }

  function setupSearch() {
    const input = $("#search-input");
    const toggle = (btn, key) => {
      btn.onclick = () => {
        search[key] = !search[key];
        btn.setAttribute("aria-pressed", String(search[key]));
        store.set("search", { case: search.case, regex: search.regex });
        runSearch();
      };
    };
    const saved = store.get("search", {});
    search.case = !!saved.case;
    search.regex = !!saved.regex;
    $("#search-case").setAttribute("aria-pressed", String(search.case));
    $("#search-regex").setAttribute("aria-pressed", String(search.regex));
    toggle($("#search-case"), "case");
    toggle($("#search-regex"), "regex");
    $("#search-scope").onclick = () => {
      search.scope = currentDir();
      renderSearchScope();
      runSearch();
    };
    $("#search-form").addEventListener("submit", (e) => {
      e.preventDefault();
      runSearch();
    });
    input.addEventListener("input", () => scheduleSearch(input.value.length < 3 ? 600 : 250));
    input.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        input.value = "";
        runSearch();
      }
    });
    renderSearchScope();
  }

  // ------------------------------------------------------------- transfers
  function downloadPath(path) {
    const a = element("a", { href: `/api/fs/download?path=${encodeURIComponent(path)}`, download: "" });
    document.body.append(a);
    a.click();
    a.remove();
  }

  /** Cloudflare caps request bodies at 100 MB: files go up in chunks. */
  const CHUNK = 32 << 20;
  const uploads = { list: [], running: false };

  /** Files dropped from the OS, folders included (as relative paths). Must
   *  be called synchronously from the drop event: the items die with it. */
  function filesFromDrop(dataTransfer) {
    const entries = [...dataTransfer.items].filter((i) => i.kind === "file").map((i) => i.webkitGetAsEntry?.()).filter(Boolean);
    if (!entries.length) return Promise.resolve([...dataTransfer.files].map((file) => ({ file: file, rel: file.name })));
    const files = [];
    const walk = async (entry, prefix) => {
      if (entry.isFile) {
        const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
        files.push({ file: file, rel: prefix + file.name });
      } else if (entry.isDirectory) {
        const reader = entry.createReader();
        let batch, count = 0;
        do {
          batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
          count += batch.length;
          for (const e of batch) await walk(e, `${prefix}${entry.name}/`);
        } while (batch.length);
        // Folders with files are created by their uploads; empty ones explicitly.
        if (!count) files.push({ dir: true, rel: prefix + entry.name });
      }
    };
    return (async () => {
      for (const e of entries) await walk(e, "");
      return files;
    })();
  }

  /** items: [{file, rel}] where rel is "name" or "folder/sub/name". */
  async function uploadInto(dir, items) {
    if (!items.length) return toast("Nothing to upload (empty folder?)");
    let listing;
    try {
      listing = await api("GET", `/api/fs?path=${encodeURIComponent(dir)}`);
    } catch (e) {
      return toast(e.message, "error");
    }
    const existing = new Set(listing.entries.map((e) => e.name));
    const topNames = [...new Set(items.map((i) => i.rel.split("/")[0]))];
    const conflicts = topNames.filter((t) => existing.has(t));
    let overwrite = false;
    if (conflicts.length) {
      const what = conflicts.length === 1 ? `“${conflicts[0]}” already exists` : `${conflicts.length} items already exist`;
      // Folders merge; only same-named files are replaced.
      overwrite = confirm(
        `${what} in ${basename(dir)}/.\n\nOK: replace files with the same names\nCancel: skip ${conflicts.length === 1 ? "this item" : "these items"}`,
      );
      if (!overwrite) items = items.filter((i) => !conflicts.includes(i.rel.split("/")[0]));
    }
    for (const item of items) {
      uploads.list.push({
        dir,
        isDir: !!item.dir,
        file: item.file || { size: 0 },
        rel: item.rel,
        overwrite,
        id: crypto.randomUUID().replaceAll("-", "").slice(0, 24),
        sent: 0,
        state: "pending",
      });
    }
    renderTransfers();
    pumpUploads();
  }

  async function pumpUploads() {
    if (uploads.running) return;
    uploads.running = true;
    let upload;
    while ((upload = uploads.list.find((x) => x.state === "pending"))) {
      upload.state = "running";
      renderTransfers();
      try {
        await uploadOne(upload);
        upload.state = "done";
      } catch (e) {
        if (upload.state !== "cancelled") {
          upload.state = "error";
          upload.error = e.message;
        }
      }
      renderTransfers();
      if (state.listings.has(upload.dir)) loadDir(upload.dir, true);
    }
    uploads.running = false;
    const failed = uploads.list.filter((x) => x.state === "error").length;
    const ok = uploads.list.filter((x) => x.state === "done").length;
    if (ok && !failed) {
      toast(`${ok} item${ok === 1 ? "" : "s"} uploaded`, "ok");
      setTimeout(() => {
        if (!uploads.running) clearTransfers();
      }, 2500);
    }
  }

  async function uploadOne(upload) {
    if (upload.isDir) return api("POST", "/api/fs/mkdirs", { dir: upload.dir, path: upload.rel });
    const size = upload.file.size;
    let offset = 0;
    do {
      const end = Math.min(size, offset + CHUNK);
      const params = new URLSearchParams({
        dir: upload.dir,
        path: upload.rel,
        id: upload.id,
        offset: String(offset),
        done: String(end >= size),
        overwrite: String(upload.overwrite),
      });
      await sendChunk(upload, `/api/fs/upload?${params}`, upload.file.slice(offset, end), offset);
      offset = end;
    } while (offset < size);
  }

  function sendChunk(upload, url, blob, base) {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      upload.xhr = xhr;
      xhr.open("PUT", url);
      xhr.upload.onprogress = (e) => {
        upload.sent = base + e.loaded;
        renderTransfersSoon();
      };
      xhr.onload = () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          upload.sent = base + blob.size;
          return resolve();
        }
        let msg = "";
        try {
          msg = JSON.parse(xhr.responseText).error;
        } catch {}
        reject(new Error(msg || (xhr.status === 403 ? "Cloudflare session expired" : `error ${xhr.status}`)));
      };
      xhr.onerror = () => reject(new Error("connection lost"));
      xhr.onabort = () => reject(new Error("cancelled"));
      xhr.send(blob);
    });
  }

  function cancelUpload(upload) {
    if (upload.state === "done") return;
    const wasRunning = upload.state === "running";
    upload.state = "cancelled";
    upload.xhr?.abort();
    if (wasRunning) {
      const params = new URLSearchParams({ dir: upload.dir, path: upload.rel, id: upload.id });
      api("DELETE", `/api/fs/upload?${params}`).catch(() => {});
    }
    renderTransfers();
  }

  function clearTransfers() {
    uploads.list = uploads.list.filter((upload) => upload.state === "pending" || upload.state === "running");
    renderTransfers();
  }

  let transfersTimer = null;
  function renderTransfersSoon() {
    transfersTimer ??= setTimeout(() => {
      transfersTimer = null;
      renderTransfers();
    }, 150);
  }

  function formatSize(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KiB", "MiB", "GiB", "TiB"];
    let i = -1;
    do {
      bytes /= 1024;
      i++;
    } while (bytes >= 1024 && i < units.length - 1);
    return `${bytes.toFixed(bytes < 10 ? 1 : 0)} ${units[i]}`;
  }

  function renderTransfers() {
    const panel = $("#transfers");
    const list = uploads.list;
    panel.hidden = !list.length;
    if (!list.length) return;
    const total = list.reduce((a, upload) => a + (upload.state === "cancelled" ? 0 : upload.file.size), 0);
    const sent = list.reduce((a, upload) => a + (upload.state === "cancelled" ? 0 : upload.state === "done" ? upload.file.size : upload.sent), 0);
    const active = list.filter((upload) => upload.state === "pending" || upload.state === "running").length;
    const errors = list.filter((upload) => upload.state === "error").length;
    const percent = total ? Math.floor((sent / total) * 100) : 100;
    $("#tr-title").textContent = active
      ? `Uploading: ${list.length - active}/${list.length} — ${percent} % (${formatSize(sent)} / ${formatSize(total)})`
      : errors
        ? `Finished with ${errors} error${errors > 1 ? "s" : ""}`
        : "Upload complete";
    $("#tr-bar").style.width = `${percent}%`;
    const shown = list.slice(-100);
    $("#tr-list").replaceChildren(
      ...shown.map((upload) => {
        const filePercent = upload.file.size ? Math.floor(((upload.state === "done" ? upload.file.size : upload.sent) / upload.file.size) * 100) : upload.state === "done" ? 100 : 0;
        const label = { pending: "pending", running: `${filePercent} %`, done: "✓", error: upload.error, cancelled: "cancelled" }[upload.state];
        return element(
          "li",
          { class: `tr-${upload.state}`, title: `${upload.dir}/${upload.rel}` },
          element("span", { class: "tr-name" }, upload.rel),
          element("span", { class: "tr-state" }, label),
          upload.state === "pending" || upload.state === "running"
            ? element("button", { class: "icon-btn sm", title: "Cancel", html: ICON.close, onclick: () => cancelUpload(upload) })
            : null,
        );
      }),
    );
  }

  /** OS file drops on an element: `target()` returns the destination folder. */
  function acceptFileDrops(el, target, highlight = () => el) {
    el.addEventListener("dragover", (e) => {
      if (!e.dataTransfer.types.includes("Files")) return;
      const dir = target(e);
      if (!dir) return;
      e.preventDefault();
      e.stopPropagation();
      e.dataTransfer.dropEffect = "copy";
      highlight(e)?.classList.add("drop-into");
    });
    el.addEventListener("dragleave", (e) => highlight(e)?.classList.remove("drop-into"));
    el.addEventListener("drop", (e) => {
      if (!e.dataTransfer.types.includes("Files")) return;
      const dir = target(e);
      highlight(e)?.classList.remove("drop-into");
      if (!dir) return;
      e.preventDefault();
      e.stopPropagation();
      filesFromDrop(e.dataTransfer).then((items) => uploadInto(dir, items)).catch((e) => toast(e.message, "error"));
    });
  }

  function pickUpload(dir, folder) {
    const input = element("input", { type: "file", multiple: true, webkitdirectory: folder || null, style: { display: "none" } });
    input.addEventListener("change", () => {
      const items = [...input.files].map((f) => ({ file: f, rel: f.webkitRelativePath || f.name }));
      input.remove();
      uploadInto(dir, items);
    });
    input.addEventListener("cancel", () => input.remove());
    document.body.append(input);
    input.click();
  }

  // ----------------------------------------------------------------- ports
  function forwardingEnabled() {
    return !!state.me.port_host_template;
  }

  /** Public URL of a forwarded port, or null when forwarding is disabled.
   *  Always a dedicated subdomain — never the UI's own origin. */
  function portUrl(port, path = "/") {
    if (!path.startsWith("/")) path = "/" + path;
    const template = state.me.port_host_template;
    return template ? `https://${template.replace("{port}", port)}${path}` : null;
  }

  function openPort(port) {
    port = Number(port);
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      toast("Invalid port", "error");
      return;
    }
    const url = portUrl(port);
    if (!url) {
      toast("Port forwarding is not configured", "error");
      return;
    }
    window.open(url, "_blank", "noopener");
  }

  async function refreshPorts() {
    try {
      state.ports = await api("GET", "/api/ports");
    } catch {
      return;
    }
    renderPorts();
  }

  function renderPorts() {
    const list = $("#port-list");
    $("#ports-count").textContent = state.ports.length || "";
    const canForward = forwardingEnabled();
    if (!state.ports.length) {
      list.replaceChildren(element("li", { class: "empty" }, "No servers listening on localhost."));
      return;
    }
    // Ports started from the user's terminals first: those are the ones they
    // are looking for. Everything else (system services…) is folded.
    const mine = state.ports.filter((port) => port.terminal);
    const others = state.ports.filter((port) => !port.terminal);
    const othersOpen = store.get("portsOthersOpen", false);
    const item = (port) => {
      // "/very/long/path/to/python3 -m http.server" → "python3 -m http.server"
      const [argv0, ...rest] = (port.cmdline || "").split(" ");
      const what = port.cmdline ? [basename(argv0), ...rest].join(" ") : port.process || "unknown process";
      const title = canForward
        ? `${port.cmdline || port.process}\n${port.cwd}\n\nOpen ${portUrl(port.port)}`
        : `${port.cmdline || port.process}\n${port.cwd}\n\nPort forwarding is not configured`;
      return element(
        "li",
        { class: "port-item", title, onclick: () => canForward && openPort(port.port) },
        element(
          "div",
          { class: "port-line" },
          element("span", { class: "port-num" }, `:${port.port}`),
          element("span", { class: "port-proc" }, port.process || "?"),
          port.terminal
            ? element(
                "button",
                {
                  class: "port-term",
                  title: `Started from terminal “${port.terminal.title}” — click to open`,
                  onclick: (e) => {
                    e.stopPropagation();
                    openTab(port.terminal.id);
                  },
                },
                icon("term"),
                port.terminal.title,
              )
            : null,
          element("span", { class: "spacer" }),
          canForward
            ? element(
                "span",
                { class: "actions" },
                element("button", {
                  class: "icon-btn sm",
                  title: "Copy link",
                  html: ICON.copy,
                  onclick: (e) => {
                    e.stopPropagation();
                    copyText(portUrl(port.port));
                  },
                }),
                element("button", {
                  class: "icon-btn sm",
                  title: "Open in new tab",
                  html: ICON.ext,
                  onclick: (e) => {
                    e.stopPropagation();
                    openPort(port.port);
                  },
                }),
              )
            : null,
        ),
        element("div", { class: "port-cmd" }, what),
        port.cwd ? element("div", { class: "port-cwd" }, prettyPath(port.cwd)) : null,
      );
    };
    const nodes = [
      ...mine.map(item),
      !mine.length ? element("li", { class: "empty" }, "No servers started from your terminals.") : null,
      others.length
        ? element(
            "li",
            {
              class: "port-group",
              onclick: () => {
                store.set("portsOthersOpen", !othersOpen);
                renderPorts();
              },
            },
            element("span", { html: ICON.chev, style: { display: "contents" } }),
            `Other processes (${others.length})`,
          )
        : null,
      ...(othersOpen ? others.map(item) : []),
    ];
    list.replaceChildren(...nodes.filter(Boolean));
    list.querySelector(".port-group")?.classList.toggle("open", othersOpen);
  }

  // ---------------------------------------------------------- context menu
  function showMenu(x, y, items) {
    const menu = $("#ctxmenu");
    const elements = [];
    let lastWasSeparator = true;
    for (const item of items) {
      if (!item) continue;
      if (item === "-") {
        if (!lastWasSeparator) elements.push(element("hr"));
        lastWasSeparator = true;
        continue;
      }
      lastWasSeparator = false;
      if (item.section) {
        elements.push(element("div", { class: "menu-label" }, item.section));
        continue;
      }
      elements.push(
        element(
          "button",
          {
            class: item.danger ? "danger" : "",
            role: "menuitem",
            onclick: () => {
              hideMenu();
              item.run();
            },
          },
          item.color ? element("span", { class: "dot", style: { background: item.color } }) : item.icon ? icon(item.icon) : element("span", { style: { width: "16px" } }),
          element("span", { style: { flex: 1 } }, item.label),
          item.hint ? element("span", { class: "muted", style: { fontSize: "11px" } }, item.hint) : null,
        ),
      );
    }
    while (elements.length && elements[elements.length - 1].tagName === "HR") elements.pop();
    menu.replaceChildren(...elements);
    menu.hidden = false;
    const bounds = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(4, Math.min(x, innerWidth - bounds.width - 4))}px`;
    menu.style.top = `${Math.max(4, Math.min(y, innerHeight - bounds.height - 4))}px`;
    menu.querySelector("button")?.focus();
  }
  function hideMenu() {
    $("#ctxmenu").hidden = true;
  }

  // --------------------------------------------------------------- palette
  let paletteItems = [];
  let paletteSelection = 0;

  function paletteSource(query) {
    const items = [
      { kind: "Action", label: "New terminal", hint: prettyPath(currentDir()), run: () => newTerminal() },
      { kind: "Action", label: "Search files", hint: "Alt+Shift+F", run: openSearch },
      { kind: "Action", label: "New command…", run: () => editButton(null) },
      { kind: "Action", label: "New file…", hint: prettyPath(currentDir()), run: () => startCreate(currentDir(), false) },
      { kind: "Action", label: "New folder…", hint: prettyPath(currentDir()), run: () => startCreate(currentDir(), true) },
      { kind: "Action", label: "Upload files…", hint: prettyPath(currentDir()), run: () => pickUpload(currentDir(), false) },
      { kind: "Action", label: "Upload folder…", hint: prettyPath(currentDir()), run: () => pickUpload(currentDir(), true) },
      ...[...state.views.values()]
        .filter((view) => view.path)
        .map((view) => ({ kind: "File", label: basename(view.path), hint: prettyPath(view.path), run: () => activate(view.id) })),
      { kind: "Action", label: state.showHidden ? "Hide hidden files" : "Show hidden files", run: toggleHidden },
      { kind: "Action", label: "Increase terminal font size", hint: `${state.fontSize + 1}px`, run: () => setFontSize(state.fontSize + 1) },
      { kind: "Action", label: "Decrease terminal font size", hint: `${state.fontSize - 1}px`, run: () => setFontSize(state.fontSize - 1) },
      { kind: "Action", label: "Explorer: return to root", hint: prettyPath(state.me.root), run: () => setViewRoot(state.me.root) },
      ...state.terms.map((terminal) => ({ kind: "Terminal", label: terminal.title, hint: prettyPath(terminal.cwd), run: () => openTab(terminal.id) })),
      ...allButtons().map((button) => ({ kind: "Command", label: button.name, hint: button.command, run: () => runButton(button) })),
      // Port entries only when forwarding is configured.
      ...(forwardingEnabled()
        ? state.ports.map((port) => ({ kind: "Port", label: `${port.port}  ${port.process || ""}`, hint: portUrl(port.port), run: () => openPort(port.port) }))
        : []),
    ];
    const q = query.trim();
    if (forwardingEnabled() && /^\d{1,5}$/.test(q)) {
      items.unshift({ kind: "Port", label: `Open port ${q}`, hint: portUrl(+q), run: () => openPort(+q) });
    }
    if (!q) return items.map((item) => ({ ...item, marks: [] }));
    return items
      .map((item) => ({ ...item, ...fuzzy(q, `${item.label}`) }))
      .filter((item) => item.score > -Infinity)
      .sort((a, button) => button.score - a.score);
  }

  /** Subsequence match; rewards consecutive and word-start matches. */
  function fuzzy(query, text) {
    const lowerText = text.toLowerCase();
    const lowerQuery = query.toLowerCase();
    const marks = [];
    let score = 0, offset = 0, previous = -2;
    for (const ch of lowerQuery) {
      if (ch === " ") continue;
      const found = lowerText.indexOf(ch, offset);
      if (found < 0) return { score: -Infinity, marks };
      score += found === previous + 1 ? 5 : 0;
      score += found === 0 || /[\s/_.-]/.test(lowerText[found - 1]) ? 3 : 0;
      score -= (found - offset) * 0.1;
      marks.push(found);
      previous = found;
      offset = found + 1;
    }
    return { score, marks };
  }

  // File names matching the palette query, fetched from the server (the
  // tree only knows the folders that were opened).
  let paletteFiles = { q: "", items: [] };
  let paletteFetch = null, paletteTimer = null;
  function fetchPaletteFiles(q) {
    clearTimeout(paletteTimer);
    paletteFetch?.abort();
    if (q.trim().length < 2) {
      paletteFiles = { q, items: [] };
      return;
    }
    paletteTimer = setTimeout(async () => {
      const controller = (paletteFetch = new AbortController());
      try {
        const response = await fetch(`/api/search/files?dir=${encodeURIComponent(state.viewRoot)}&q=${encodeURIComponent(q)}`, { signal: controller.signal });
        if (!response.ok) return;
        const hits = await response.json();
        if ($("#palette-input").value !== q) return;
        paletteFiles = { q, items: hits };
        renderPalette({ keepSelection: true });
      } catch {}
    }, 120);
  }

  function openPalette() {
    hideMenu();
    $("#palette").hidden = false;
    const input = $("#palette-input");
    input.value = "";
    paletteFiles = { q: "", items: [] };
    renderPalette();
    input.focus();
  }
  function closePalette() {
    $("#palette").hidden = true;
    state.views.get(state.active)?.focus();
  }
  function renderPalette({ keepSelection = false } = {}) {
    const q = $("#palette-input").value;
    const files = paletteFiles.q === q
      ? paletteFiles.items.map((f) => ({ kind: "File", label: f.rel, marks: f.marks, hint: "", run: () => openEditor(f.path) }))
      : [];
    paletteItems = [...paletteSource(q).slice(0, 30), ...files].slice(0, 80);
    if (!keepSelection) paletteSelection = 0;
    const list = $("#palette-list");
    paletteSelection = Math.min(paletteSelection, Math.max(0, paletteItems.length - 1));
    if (!paletteItems.length) {
      list.replaceChildren(element("li", { class: "none" }, q.trim().length >= 2 && paletteFiles.q !== q ? "Searching…" : "No results"));
      return;
    }
    list.replaceChildren(
      ...paletteItems.map((item, i) => {
        const label = element("span", { class: "label" });
        const marks = new Set(item.marks);
        // marks are UTF-16 offsets (String indices), like indexOf returns.
        let at = 0;
        for (const c of item.label) {
          label.append(marks.has(at) ? element("mark", {}, c) : c);
          at += c.length;
        }
        return element(
          "li",
          {
            class: i === paletteSelection ? "sel" : "",
            role: "option",
            onmousemove: () => setPaletteSelection(i),
            onclick: () => runPalette(i),
          },
          element("span", { class: "kind" }, item.kind),
          label,
          item.hint ? element("span", { class: "hint" }, item.hint) : null,
        );
      }),
    );
  }
  function setPaletteSelection(i) {
    const rows = $("#palette-list").children;
    if (!paletteItems.length) return;
    paletteSelection = (i + paletteItems.length) % paletteItems.length;
    for (let j = 0; j < rows.length; j++) rows[j].classList.toggle("sel", j === paletteSelection);
    rows[paletteSelection]?.scrollIntoView({ block: "nearest" });
  }
  function runPalette(i) {
    const item = paletteItems[i];
    if (!item) return;
    $("#palette").hidden = true;
    item.run();
  }

  // -------------------------------------------------------------- settings
  function setFontSize(px) {
    state.fontSize = Math.max(9, Math.min(24, px));
    store.set("fontSize", state.fontSize);
    for (const view of state.views.values()) view.setFontSize(state.fontSize);
  }

  function toggleHidden() {
    state.showHidden = !state.showHidden;
    $("#fs-hidden").classList.toggle("on", state.showHidden);
    saveTree();
    renderTree();
  }

  function toggleSidebar() {
    if (window.matchMedia("(max-width: 760px)").matches) document.body.classList.toggle("sidebar-open");
    else {
      document.body.classList.toggle("no-sidebar");
      store.set("sidebar", !document.body.classList.contains("no-sidebar"));
    }
    requestAnimationFrame(() => state.views.get(state.active)?.fitNow());
  }

  // -------------------------------------------------------- mobile keys
  let stickyCtrl = false;
  function applyStickyCtrl(data) {
    if (!stickyCtrl || data.length !== 1) return data;
    stickyCtrl = false;
    $('#mobile-keys [data-key="ctrl"]').classList.remove("on");
    const charCode = data.toUpperCase().charCodeAt(0);
    return charCode >= 64 && charCode <= 95 ? String.fromCharCode(charCode - 64) : data;
  }
  function setupMobileKeys() {
    if (!isTouch) return;
    const bar = $("#mobile-keys");
    bar.hidden = true; // shown by activate() when a terminal is in front
    setupTouchHelpers();
    const seq = { esc: "\x1b", tab: "\t", up: "\x1b[A", down: "\x1b[B", right: "\x1b[C", left: "\x1b[D", pipe: "|", tilde: "~", slash: "/" };
    bar.addEventListener("pointerdown", (e) => e.preventDefault()); // keep the keyboard open
    bar.addEventListener("click", (e) => {
      const key = e.target.closest("button")?.dataset.key;
      const view = state.views.get(state.active);
      if (!key || view?.kind !== "term") return;
      if (key === "ctrl") {
        stickyCtrl = !stickyCtrl;
        e.target.classList.toggle("on", stickyCtrl);
      } else view.send(seq[key]);
      view.term.focus();
    });
  }

  function setupTouchHelpers() {
    // iOS ignores interactive-widget=resizes-content: the on-screen keyboard
    // overlays the page. Size the app to the visual viewport instead, so the
    // terminal and the key bar stay above the keyboard.
    const viewport = window.visualViewport;
    if (viewport) {
      const fit = () => {
        document.documentElement.style.setProperty("--app-h", `${viewport.height}px`);
        window.scrollTo(0, 0);
      };
      viewport.addEventListener("resize", fit);
      fit();
    }
    // iOS Safari never fires contextmenu on a long press; Android does. Open
    // our menus after a still 550 ms press unless the browser already did.
    let timer = null, nativeFired = false, start = null, opened = false;
    document.addEventListener("contextmenu", () => (nativeFired = opened = true), true);
    document.addEventListener(
      "touchstart",
      (e) => {
        if (e.touches.length !== 1 || e.target.closest(".view, input, textarea")) return;
        const touch = e.touches[0];
        start = { x: touch.clientX, y: touch.clientY, target: e.target };
        nativeFired = opened = false;
        clearTimeout(timer);
        timer = setTimeout(() => {
          if (nativeFired || !start) return;
          start.target.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: start.x, clientY: start.y }));
          start = null;
          opened = true;
        }, 550);
      },
      { passive: true },
    );
    const cancel = (e) => {
      if (!start) return;
      const touch = e.touches?.[0];
      if (e.type === "touchmove" && touch && Math.hypot(touch.clientX - start.x, touch.clientY - start.y) < 10) return;
      clearTimeout(timer);
      start = null;
    };
    for (const ev of ["touchmove", "touchcancel"]) document.addEventListener(ev, cancel, { passive: true });
    document.addEventListener(
      "touchend",
      (e) => {
        cancel(e);
        // Lifting the finger after a long press would emulate a click, which
        // closes the menu that just opened and activates what was under it.
        if (opened) {
          e.preventDefault();
          opened = false;
        }
      },
      { passive: false },
    );
  }

  /** Fullscreen + Keyboard Lock: the browser then hands every key to the
   *  page, including the ones it normally keeps (Ctrl+W, Ctrl+T, Ctrl+N…).
   *  Holding Escape leaves fullscreen. Chromium-based browsers only. */
  function setupKeyboardLock() {
    const btn = $("#kbd-lock");
    if (!navigator.keyboard?.lock || !document.documentElement.requestFullscreen) {
      btn.hidden = true;
      return;
    }
    btn.onclick = async () => {
      if (document.fullscreenElement) return document.exitFullscreen();
      try {
        await document.documentElement.requestFullscreen();
        await navigator.keyboard.lock();
        toast("All keys go to the terminal. Hold Escape to exit.", "ok");
        state.views.get(state.active)?.focus();
      } catch (e) {
        toast(`Cannot enter fullscreen: ${e.message}`, "error");
      }
    };
    document.addEventListener("fullscreenchange", () => {
      const on = !!document.fullscreenElement;
      btn.classList.toggle("on", on);
      if (!on) navigator.keyboard.unlock();
    });
  }

  // ------------------------------------------------------------- shortcuts
  /** App shortcuts never steal keys from a focused terminal: there, only ⌘K
   *  (macOS) is ours. Alt+Shift shortcuts work everywhere else. */
  function isAppShortcut(e) {
    if (e.type !== "keydown") return false;
    const inWorkspaceView = e.target instanceof Element && e.target.closest(".view");
    if (e.altKey && e.shiftKey && !e.ctrlKey && !e.metaKey && !inWorkspaceView)
      return /^(KeyT|KeyW|KeyP|KeyB|KeyE|KeyF|ArrowLeft|ArrowRight|Digit[1-9])$/.test(e.code);
    return isMac && e.metaKey && !e.altKey && !e.ctrlKey && e.code === "KeyK";
  }

  function onGlobalKeydown(e) {
    if (!$("#palette").hidden) return; // the palette handles its own keys
    if (e.key === "Escape" && !$("#ctxmenu").hidden) {
      hideMenu();
      e.preventDefault();
      return;
    }
    if (!isAppShortcut(e)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "KeyK" || e.code === "KeyP") return openPalette();
    if (e.code === "KeyT") return newTerminal();
    if (e.code === "KeyW") return state.active && closeTab(state.active);
    if (e.code === "KeyB") return toggleSidebar();
    if (e.code === "KeyF") return openSearch();
    if (e.code === "ArrowLeft") return cycleTab(-1);
    if (e.code === "ArrowRight") return cycleTab(1);
    if (e.code === "KeyE") {
      document.body.classList.remove("no-sidebar");
      if (window.matchMedia("(max-width: 760px)").matches) document.body.classList.add("sidebar-open");
      $("#panel-files").classList.remove("collapsed");
      if (!state.selected) {
        const first = visibleRows()[0];
        if (first) select(first.path, first.entry.dir);
      }
      return $("#tree").focus();
    }
    const n = e.code.match(/^Digit(\d)$/);
    if (n && state.order[n[1] - 1]) activate(state.order[n[1] - 1]);
  }

  // ----------------------------------------------------------------- setup
  function setupLayout() {
    const sidebarW = store.get("sidebarW", 280);
    document.documentElement.style.setProperty("--sidebar-w", `${sidebarW}px`);
    if (store.get("sidebar", true) === false) document.body.classList.add("no-sidebar");

    const resizer = $("#resizer");
    resizer.addEventListener("pointerdown", (e) => {
      resizer.setPointerCapture(e.pointerId);
      resizer.classList.add("dragging");
      const startW = parseInt(getComputedStyle(document.documentElement).getPropertyValue("--sidebar-w"));
      let collapse = false;
      const move = (ev) => {
        // Dragged far to the left: fold the column instead of shrinking it.
        collapse = ev.clientX < 120;
        resizer.classList.toggle("will-collapse", collapse);
        const w = Math.max(180, Math.min(innerWidth * 0.6, ev.clientX));
        document.documentElement.style.setProperty("--sidebar-w", `${w}px`);
      };
      const up = () => {
        resizer.classList.remove("dragging", "will-collapse");
        resizer.removeEventListener("pointermove", move);
        resizer.removeEventListener("pointerup", up);
        if (collapse) {
          document.documentElement.style.setProperty("--sidebar-w", `${startW}px`);
          toggleSidebar();
          return;
        }
        store.set("sidebarW", parseInt(getComputedStyle(document.documentElement).getPropertyValue("--sidebar-w")));
        state.views.get(state.active)?.fitNow();
      };
      resizer.addEventListener("pointermove", move);
      resizer.addEventListener("pointerup", up);
    });

    const collapsed = store.get("collapsed", {});
    for (const btn of document.querySelectorAll(".panel-toggle")) {
      const panel = btn.closest(".panel");
      if (btn.dataset.panel in collapsed) panel.classList.toggle("collapsed", !!collapsed[btn.dataset.panel]);
      btn.addEventListener("click", () => {
        panel.classList.toggle("collapsed");
        collapsed[btn.dataset.panel] = panel.classList.contains("collapsed");
        store.set("collapsed", collapsed);
        if (btn.dataset.panel === "ports" && !collapsed.ports) refreshPorts();
      });
    }

    // Fit the active terminal whenever its container changes size.
    let fitTimer;
    new ResizeObserver(() => {
      clearTimeout(fitTimer);
      fitTimer = setTimeout(() => state.views.forEach((view) => view.fitNow()), 30);
    }).observe($("#views"));

    $("#toggle-sidebar").onclick = toggleSidebar;
    $("#open-palette").onclick = openPalette;
    $("#empty-palette").onclick = openPalette;
    $("#empty-new").onclick = () => newTerminal();
    $("#tab-new").onclick = () => newTerminal();
    $("#terms-new").onclick = () => newTerminal();
    $("#fs-refresh").onclick = refreshTree;
    $("#fs-hidden").onclick = toggleHidden;
    $("#fs-collapse").onclick = () => {
      state.expanded.clear();
      saveTree();
      renderTree();
    };
    $("#fs-up").onclick = () => setViewRoot(parentOf(state.viewRoot));
    $("#fs-newfile").onclick = () => startCreate(currentDir(), false);
    $("#fs-newfolder").onclick = () => startCreate(currentDir(), true);
    $("#fs-upload").onclick = (e) => {
      const r = e.currentTarget.getBoundingClientRect();
      showMenu(r.left, r.bottom + 2, [
        { section: `To ${prettyPath(currentDir())}` },
        { label: "Upload files…", icon: "upload", run: () => pickUpload(currentDir(), false) },
        { label: "Upload folder…", icon: "upload", run: () => pickUpload(currentDir(), true) },
      ]);
    };
    // OS drops: on a folder row into it, on a file row into its folder,
    // elsewhere in the tree into the root of the view.
    const rowOf = (e) => e.target.closest?.("#tree .node[data-path]:not(.create-row)");
    acceptFileDrops(
      $("#panel-files .panel-body"),
      (e) => {
        const row = rowOf(e);
        if (!row) return state.viewRoot;
        return row.classList.contains("dir") ? row.dataset.path : parentOf(row.dataset.path);
      },
      (e) => rowOf(e) || $("#tree"),
    );
    // A file dropped anywhere else must not make the browser navigate to it.
    for (const ev of ["dragover", "drop"])
      window.addEventListener(ev, (e) => e.dataTransfer?.types.includes("Files") && e.preventDefault());
    $("#tr-close").onclick = clearTransfers;
    for (const b of document.querySelectorAll("#rail [data-panel]"))
      b.onclick = () => {
        toggleSidebar();
        const panel = $(`#panel-${b.dataset.panel}`);
        panel.classList.remove("collapsed");
        if (b.dataset.panel === "files") $("#tree").focus();
        if (b.dataset.panel === "search") openSearch();
      };
    $("#resizer").addEventListener("dblclick", toggleSidebar);
    $("#sidebar-fold").addEventListener("pointerdown", (e) => e.stopPropagation());
    $("#sidebar-fold").onclick = toggleSidebar;
    $("#rail-unfold").onclick = toggleSidebar;
    $("#ports-refresh").onclick = refreshPorts;
    $("#port-form").addEventListener("submit", (e) => {
      e.preventDefault();
      openPort($("#port-input").value.trim());
    });
    $("#tree").addEventListener("keydown", treeKeydown);
    $("#palette-kbd").textContent = isMac ? "⌘K" : "Alt+Shift+P";

    $("#palette").addEventListener("mousedown", (e) => e.target.id === "palette" && closePalette());
    $("#palette-input").addEventListener("input", () => {
      renderPalette();
      fetchPaletteFiles($("#palette-input").value);
    });
    $("#palette-input").addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") setPaletteSelection(paletteSelection + 1);
      else if (e.key === "ArrowUp") setPaletteSelection(paletteSelection - 1);
      else if (e.key === "Enter") runPalette(paletteSelection);
      else if (e.key === "Escape") closePalette();
      else return;
      e.preventDefault();
    });

    document.addEventListener("mousedown", (e) => {
      if (!$("#ctxmenu").hidden && !e.target.closest("#ctxmenu")) hideMenu();
    });
    window.addEventListener("blur", hideMenu);
    window.addEventListener("keydown", onGlobalKeydown, true);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) {
        refreshTerms();
        state.views.get(state.active)?.tab.classList.remove("activity");
      }
    });
    setupButtonDialog();
    setupSearch();
    setupMobileKeys();
    setupKeyboardLock();
  }

  async function init() {
    try {
      state.me = await api("GET", "/api/me");
    } catch (e) {
      $("#empty-title").textContent = "Cannot connect";
      $("#empty .muted").textContent = e.message;
      return;
    }
    document.title = `${state.me.name} · codeenv`;
    $("#host-name").textContent = state.me.name;
    $("#empty-title").textContent = state.me.name;
    $("#user").textContent = state.me.email;
    $("#user").title = `Signed in as ${state.me.email}`;
    state.fontSize = store.get("fontSize", window.matchMedia("(pointer: coarse)").matches ? 12 : 13);
    // The "open a port" box only makes sense when forwarding is configured.
    if (!forwardingEnabled()) {
      const form = $("#port-form");
      if (form) form.hidden = true;
    }

    setupLayout();
    renderButtons();

    // Explorer
    const tree = store.get("tree", {});
    state.showHidden = !!tree.hidden;
    $("#fs-hidden").classList.toggle("on", state.showHidden);
    state.viewRoot = tree.viewRoot && isUnder(tree.viewRoot, state.me.root) ? tree.viewRoot : state.me.root;
    renderSearchScope();
    state.expanded = new Set((tree.expanded || []).filter((p) => isUnder(p, state.viewRoot)));
    await loadDir(state.viewRoot);
    await Promise.all([...state.expanded].map((p) => loadDir(p).then((l) => l.error && state.expanded.delete(p))));
    renderTree();

    // Terminals: reopen the tabs of the last visit that still exist.
    await refreshTerms();
    const saved = store.get("tabs", { order: [], active: null });
    const alive = new Set(state.terms.map((t) => t.id));
    for (const id of saved.order) {
      if (alive.has(id)) openTab(id, { focus: false });
      else if (id.startsWith("edit:") && isUnder(id.slice(5), state.me.root)) openEditor(id.slice(5), { show: false, focus: false });
    }
    if (state.order.length) activate(state.order.includes(saved.active) ? saved.active : state.order[0]);
    $("#empty").hidden = state.order.length > 0;

    const remeasure = () => state.views.forEach((view) => view.remeasure?.());
    document.fonts?.ready.then(remeasure);
    document.fonts?.addEventListener("loadingdone", remeasure);

    refreshPorts();
    setInterval(() => !document.hidden && refreshTerms(), 4000);
    setInterval(() => !document.hidden && !$("#panel-ports").classList.contains("collapsed") && refreshPorts(), 5000);
  }

  init();
})();
