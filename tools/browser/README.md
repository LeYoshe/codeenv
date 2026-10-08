# Browser checks and demo media

The scripts run the real codeenv binary with disposable data. They never use
your projects or production configuration. Requirements: Node.js 22+, Python 3,
and a debug build from `cargo build --locked` at the repository root.

```sh
npm ci
npx playwright install chromium
npm test
```

On Linux, install browser system dependencies with
`npx playwright install --with-deps chromium` if needed. CI uses this command.

`CODEENV_BINARY` can select another compiled binary. `CHROME_PATH` can select
an existing Chromium or Chrome executable instead of Playwright's browser.
No machine-specific browser paths are stored in the scripts.

## Record the site demo

```sh
npm run capture
```

This creates `site/media/workspace.png`, `terminal.png`, `search.png`, and
`demo.webm`. It starts a local daemon and server, uses a shell without profile
files, runs real Python tests, edits a file, and searches the sample project.
Processes and temporary data are cleaned up when it finishes.

Create an MP4 alternative with FFmpeg, from the repository root:

```sh
ffmpeg -y -i site/media/demo.webm -an -c:v libx264 -crf 25 \
  -pix_fmt yuv420p -movflags +faststart site/media/demo.mp4
```

The website uses a video element with controls and a poster, not an animated
GIF. It does not autoplay or download the full clip on page load. Both formats
have no audio; a text description appears beside the recording.

The GitHub README uses a looping GIF generated from the same recording:

```sh
ffmpeg -y -i site/media/demo.mp4 \
  -filter_complex '[0:v]fps=8,scale=1080:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle' \
  -loop 0 site/media/demo.gif
```

Keep the GIF short and small enough to load directly in GitHub. Its README
link opens the site's video player, which has playback controls.

Before committing, inspect every screenshot and the entire video for private
paths, account details, browser extensions, or other accidental personal data.
Sample names and the temporary `/tmp/codeenv-demo-…` paths are intentional.
