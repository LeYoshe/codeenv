# Releases and site maintenance

This guide is for maintainers of `LeYoshe/codeenv`.

## Repository settings

These settings cannot be enabled just by adding files to the repository:

1. Enable Issues. The bug and feature forms in `.github/ISSUE_TEMPLATE/`
   appear automatically; the pull request template does too.
2. Enable Actions. If the organization restricts actions, allow the official
   `actions/*` actions used in the workflows.
3. In **Settings → Pages**, choose **GitHub Actions** as the source.
4. Enable **Private vulnerability reporting** in the security settings, so
   the link in `SECURITY.md` works. Enable secret scanning and push protection
   where available.
5. Protect `main`. Require `linux-amd64`, `linux-arm64`, `macos-arm64`, and
   `Rust coverage` to pass, with review before merging.
6. Check the `github-pages` environment. Its deployment branch rules should
   allow `main`.

Dependabot version update PRs are paused with `open-pull-requests-limit: 0`
in `.github/dependabot.yml`. Set a positive limit for each ecosystem when
repository setup is complete.

The site workflow deploys only `site/`, not the running terminal app. It has
no backend and needs no Cloudflare setup. Relative asset paths work under
`/codeenv/`. Run the **GitHub Pages** workflow manually if the initial push
happened before Pages was enabled. Automatic deployment runs only for a public
repository. For a private repository, Pages requires a supported GitHub plan;
after enabling it, deploy manually.

## CI artifacts

`CI` runs for pushes to `main`, pull requests, version tags, and manual runs.
All three builds use native runners and the Rust version in
`rust-toolchain.toml`:

| Archive suffix | Runner | Rust target |
|---|---|---|
| `linux-amd64` | `ubuntu-22.04` | `x86_64-unknown-linux-gnu` |
| `linux-arm64` | `ubuntu-22.04-arm` | `aarch64-unknown-linux-gnu` |
| `macos-arm64` | `macos-15` | `aarch64-apple-darwin` |

The Linux archives use glibc 2.35 or later. They are not static musl binaries.
The macOS build targets macOS 14 or later and is not notarized with Apple.
Native tests run on every target. Browser regression tests run on Linux AMD64.
The separate `Rust coverage` job publishes an HTML report and JSON summary
in the `rust-coverage` artifact; see [tests and coverage](testing.md).

Each artifact contains a `.tar.gz` archive and its SHA-256 checksum. Artifacts
are retained for 14 days. The archive includes the binary, docs, configuration
examples, project license, and dependency notices. Packaging fails if a
resolved dependency has no license text; investigate its upstream package
rather than dropping its attribution.

## Publish a version

1. Update `version` in `Cargo.toml`, run `cargo check` to update `Cargo.lock`,
   and include any protocol or deployment changes in the docs.
2. Merge after CI passes.
3. Tag that commit with the matching version and push the tag:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

4. Wait for all three builds and the coverage job. CI creates a **draft release**
   with the archives, `SHA256SUMS`, and generated release notes. A tag that does not match
   `Cargo.toml` is rejected.
5. Review the notes and downloads, then publish the draft in GitHub Releases.

Rerunning a tag workflow can update a draft, but it refuses to replace assets
of an already published release. Use a new version for a changed release.
No personal access token is needed; jobs use scoped `GITHUB_TOKEN` permissions.

On Linux, verify a download with `sha256sum --check SHA256SUMS`. On macOS,
use `shasum -a 256 --check SHA256SUMS`. Download all listed archives to check
the whole file, or select the line for your archive first.

## Maintain the site

Edit `site/index.html` and `site/style.css`. Preview locally:

```sh
python3 -m http.server 8000 --directory site --bind 127.0.0.1
```

Open `http://127.0.0.1:8000`. The site has no build tool or external font service.
Check desktop and mobile widths, links, screenshots, and video playback before
merging. Pushes affecting `site/` deploy through the Pages workflow.

Media is recorded from the real app with sample data. Follow
[the recording instructions](../tools/browser/README.md) when UI changes make
screenshots outdated. Keep the text description beside the video in sync.
