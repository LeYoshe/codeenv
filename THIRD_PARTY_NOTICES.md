# Third-party software

codeenv's own code is MIT licensed. Dependencies retain their own licenses
and copyright notices.

## Browser libraries

- xterm.js and its fit, web-links, Unicode 11, and WebGL addons are distributed
  under the MIT license. See `web/vendor/LICENSE-xterm` in the source tree or
  `licenses/LICENSE-xterm` in release archives.
- CodeMirror, Lezer, and the other packages included in the editor bundle have
  their notices in `web/vendor/LICENSE-codemirror` (or
  `licenses/LICENSE-codemirror` in release archives). `npm run build` in
  `tools/codemirror/` regenerates these notices from the packages actually
  included in the bundle.

Do not remove these notices when redistributing the bundled browser libraries.

## Rust dependencies

`Cargo.lock` records the Rust dependency versions. Release packaging collects
license files from the resolved dependencies for each target, including build
dependencies, into `licenses/RUST-DEPENDENCIES.txt` inside each archive.

The license of codeenv does not replace these dependency licenses. See each
included notice for its terms and attribution.
