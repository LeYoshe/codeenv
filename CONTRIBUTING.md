# Contributing

Bug reports, documentation fixes, and focused pull requests are welcome.
For a substantial feature, open an issue first to agree on the problem and scope.

## Report a problem

Use the [bug report form](https://github.com/leyoshe/codeenv/issues/new?template=bug_report.yml).
Include your codeenv version, OS and architecture, browser, steps to reproduce,
and what you expected. Remove tokens, cookies, private paths, and credentials
from logs. Report security problems privately as described in [SECURITY.md](SECURITY.md).

Use the [feature request form](https://github.com/leyoshe/codeenv/issues/new?template=feature_request.yml)
for a missing capability. Explain the task you want to accomplish and any
workaround you use today.

## Run locally

Install Rust using the version in [rust-toolchain.toml](rust-toolchain.toml)
and Node.js 22 or later for JavaScript checks. Project scripts need Python
3.11 or later. Native dependencies need a C/C++ compiler, Make, and CMake
(`build-essential cmake` on Debian/Ubuntu; Xcode Command Line Tools and
`brew install cmake` on macOS). Fork the repository, clone your fork,
and create a branch:

```sh
git switch -c fix/short-description
cargo build --locked
```

Follow the [README's local setup](README.md#development) to start the app.
Debug builds read frontend files from disk, so reloading picks up UI changes.
There is no frontend build step unless you change the CodeMirror bundle.

## Check your changes

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
node --check web/app.js
node --check tools/codemirror/entry.js
python3 scripts/check-project.py
```

Try affected features in a browser. For terminal changes, check typing,
resizing, reconnecting, and restarting the web server. For file changes,
check saving, conflicts, and permissions. Add a regression test when fixing
a bug that an automated test can reproduce.

CI tests and builds on Linux AMD64, Linux ARM64, and macOS ARM64. Linux process
inspection needs Linux validation even when a change passes on macOS.

## Browser checks and demo recordings

```sh
cargo build --locked
cd tools/browser
npm ci
npx playwright install chromium
npm test
```

See [tools/browser/README.md](tools/browser/README.md) for recording screenshots
and video with disposable sample data.

## Keep changes readable

Use English for all project text, including UI labels, errors, comments, and
documentation. Prefer familiar names and short explanations. Avoid unrelated
formatting changes, new abstractions without a concrete use, and dependencies
that duplicate existing code. See [CLAUDE.md](CLAUDE.md) for architecture constraints.

Do not edit minified vendor files by hand. To rebuild CodeMirror and its notices:

```sh
cd tools/codemirror
npm ci
npm run build
```

Commit the source, lockfile, generated bundle, and license notices together.
Keep required third-party copyright notices intact.

## Send a pull request

Explain the problem, what changes for users, and how you checked it. Include
screenshots for visible UI changes. Keep each pull request focused enough to
review independently. Do not include credentials or personal configuration.

Contributions are distributed under the project's [MIT license](LICENSE).
No contributor agreement is required.

See [Tests and coverage](docs/testing.md) for integration scenarios, the coverage
report, and known gaps. Add a regression test when fixing a bug.
