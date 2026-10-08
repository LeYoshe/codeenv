# Browser libraries

These files are served locally and embedded in the codeenv binary.

## CodeMirror

`codemirror.js` and `LICENSE-codemirror` are generated from the versions in
`tools/codemirror/package-lock.json`. To update them:

```sh
cd tools/codemirror
npm update
npm run build
npm run check
```

## xterm.js

The terminal files come from the following npm releases:

| Package | Version | Files copied from the package |
|---|---|---|
| `@xterm/xterm` | 6.0.0 | `lib/xterm.js`, `css/xterm.css`, `LICENSE` |
| `@xterm/addon-fit` | 0.11.0 | `lib/addon-fit.js` |
| `@xterm/addon-unicode11` | 0.9.0 | `lib/addon-unicode11.js` |
| `@xterm/addon-web-links` | 0.12.0 | `lib/addon-web-links.js` |
| `@xterm/addon-webgl` | 0.19.0 | `lib/addon-webgl.js` |

Use `npm pack PACKAGE@VERSION` to download a release. Copy the files above
into this directory, collecting each package's license in `LICENSE-xterm`.
Keep the files unmodified and check the add-ons' license notices when updating.
All five packages currently share the MIT license in `LICENSE-xterm`.
Update this table, rebuild codeenv, and run the browser checks after changes.
Dependabot does not track these copied files.
