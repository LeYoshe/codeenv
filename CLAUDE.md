# Working on codeenv

This file covers the basics of changing the project, by hand or with a coding
assistant. The [README](README.md) explains usage; [docs/](docs/) contains the
guides and references.

## Check a change

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
node --check web/app.js
```

Use the README's local `insecure` configuration to try the interface. Check
affected features in a browser: terminal reconnection, editing, search, or
transfers. Linux-specific code (`/proc` inspection, signals) also needs
validation on Linux.

Debug builds read `web/` from disk. Release builds embed it. The frontend has
no build step except for the CodeMirror bundle: run `npm ci && npm run build`
in `tools/codemirror/`.

## Keep these constraints in mind

- The `codeenv ptyd` daemon keeps terminals alive across web server restarts.
  Keep `KillMode=process` in the systemd service.
- An incompatible daemon message change requires incrementing `PROTOCOL`.
  This requires a daemon restart and closes its terminals.
- Requests pass through host checks, authentication, and optional port
  forwarding. API routes also check the origin.
- Each forwarded port uses a separate subdomain. An app served on the UI's
  origin could call its API.
- Keep asset paths in HTML in the form `"/assets/…"`: the server recognizes
  them and adds content hashes.
- Explorer operations use the path resolution functions in `src/files.rs`.
  Use `open_regular` to read files; it rejects FIFOs and devices. Shells keep
  the permissions of the service's Unix account.
- Files and terminal output may contain hostile content. Keep control
  character filtering for inserted paths and the CSP policy for SVG previews.

## Write plainly

Use short sentences and familiar words. Comments explain a constraint or a
decision; they should not repeat each instruction. Name variables by their
role: `request`, `terminal`, `listing`, `upload`. Keep common abbreviations
such as `pid`, `cwd`, and `ws`.

Write all project text in English: documentation, code comments, log messages,
interface labels, tooltips, accessibility text, and errors.

Remove unused parameters and repetition. Add an abstraction when it makes
existing code easier to follow. Preserve third-party files in `web/vendor/`.

Use `files::http_err` when an error needs a specific HTTP status. Document
actual behavior and its limits. Avoid absolute guarantees that the code
cannot support.
