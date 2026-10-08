# Tests and coverage

Run the Rust suite with `cargo test --locked`. It includes unit tests and
integration tests that start the production router on a loopback TCP socket.
Tests use temporary files and local servers, not a Cloudflare account.

## What the tests exercise

- Access JWT validation and authentication on HTTP and WebSocket routes.
- Host restrictions, cross-origin writes, cross-site reads, and WebSocket origins.
- Refusal to list, rename, delete, or attach another user's terminal. This test
  uses a daemon fixture that only accepts list requests; the browser suite
  exercises a real daemon separately.
- HTTP forwarding to a local server, request bodies and query strings,
  credential stripping, cookie scope, forbidden ports, and unavailable servers.
- WebSocket forwarding with text, binary, and close frames.
- File confinement, symlinks, edit limits, stale versions, and concurrent saves.
- Chunked uploads, retries after a failed body stream, cancellation, parallel
  directory creation, destination conflicts, and ZIP downloads.
- Preference persistence, invalid updates, and corrupt stored JSON.

The browser suite runs against the release binary on Linux AMD64. It checks
terminal reconnection, editor interactions, transfers, search, and selected
error states. See [browser checks](../tools/browser/README.md).

## Coverage report

CI's **Rust coverage** job publishes an HTML report and `summary.json` in the
`rust-coverage` artifact. Line, function, and region totals appear in its job
summary. To reproduce locally:

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.9.1 --locked
cargo llvm-cov --locked --html --output-dir coverage --ignore-filename-regex integration_tests.rs
```

Open `coverage/html/index.html`. The integration test fixture file is excluded
from the report. The metric covers the Rust test run; it does not include the
separate browser run or JavaScript coverage. There is no arbitrary percentage
gate: use uncovered lines to choose meaningful tests, not to inflate a score.

## Remaining limits

The suite does not simulate every disk failure (full disk, failed fsync), live
Cloudflare key rotation, all daemon failures, or slow-client backpressure.
Browser automation currently uses Chromium only. It is not a load test or a
security audit.

Editor saves serialize version checks and replacements within one codeenv
process. External editors, separate codeenv processes, and uploads do not use
that lock. Conflict detection cannot guarantee exclusion against those writers.
Parallel upload tests use distinct upload IDs; concurrent requests for the same
upload ID are not yet covered.
