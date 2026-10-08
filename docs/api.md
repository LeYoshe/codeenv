# HTTP API

The web interface uses this API to manage terminals and files. Scripts can
use it too. The API follows the interface's needs and may change with codeenv.

## Send a request

In production, every request must carry a valid Cloudflare Access token.
Local development in `insecure` mode uses the configured identity. Requested
hostnames must be accepted by `hosts` or match a forwarded port.

To modify data or open a WebSocket, also send an `Origin` header whose hostname
and port match the request. Reads marked with `Sec-Fetch-Site: cross-site` or
`same-site` are rejected.

This example assumes a local instance in `insecure` mode on port 7690:

```sh
curl http://127.0.0.1:7690/api/me
curl -X POST http://127.0.0.1:7690/api/terminals \
  -H 'Origin: http://127.0.0.1:7690' \
  -H 'Content-Type: application/json' \
  -d '{"title":"Project","cwd":"."}'
```

Request bodies are JSON except for file uploads, which use raw bytes.
Application errors use `{"error":"message"}`. Authentication failures, body
parsing errors, and some routing errors may return text: check the status
and content type before decoding a response.

## Identity: `GET /api/me`

Returns the user, interface settings, and saved commands:

```json
{
  "email": "dev@example.com",
  "name": "vps1",
  "root": "/home/dev",
  "home": "/home/dev",
  "shell": "/bin/bash",
  "global_buttons": [],
  "buttons": [],
  "port_host_template": "p{port}-vps1.example.com"
}
```

`port_host_template` is `null` when port forwarding is disabled.

## Files

The `path` and `dir` parameters accept absolute paths or paths relative to
`root`. An empty path means the root. The explorer rejects paths that leave
this root after resolving symlinks.

| Request | Parameters or body | Result |
|---|---|---|
| `GET /api/fs` | `?path=...` | Directory: `path`, `entries`, `truncated` |
| `GET /api/fs/file` | `?path=...` | File: `path`, `content`, `version`, `readonly` |
| `PUT /api/fs/file` | `{"path":"...","content":"...","version":"..."}` | New `version` |
| `POST /api/fs/create` | `{"path":"...","dir":false}` | Created path; `dir:true` creates a directory |
| `POST /api/fs/mkdirs` | `{"dir":"...","path":"a/b/c"}` | Creates missing directories and returns `path` |
| `POST /api/fs/rename` | `{"from":"...","to":"..."}` | New `path` |
| `POST /api/fs/delete` | `{"path":"..."}` | `{}`; deletes directories recursively |

Each directory entry has `name`, `dir`, `link`, `size` in bytes, and `mtime`
in seconds since January 1, 1970. `truncated:true` means the listing reached
its limit of 5,000 entries.

The editor reads and writes UTF-8 text up to 5 MiB. To save, send back the
`version` received when reading. If the content on disk has changed, the
server returns `409`. Omitting `version` requests replacement without that
check. Saving writes a temporary file, then renames it. Editor saves within
one codeenv process serialize this check and replacement. External writers
and uploads do not take the editor lock.

Renaming rejects an existing destination. Renaming or deleting a symlink acts
on the link, not its target. These operations do not lock out concurrent
filesystem access by other programs.

## Uploads and downloads

### Upload: `PUT /api/fs/upload`

Send a raw chunk in the body with these URL parameters:

| Parameter | Meaning |
|---|---|
| `dir` | Destination directory |
| `path` | Relative path, such as `project/src/main.rs` |
| `id` | 1–40 ASCII letters, digits, or hyphens; use the same ID for every chunk |
| `offset` | Bytes already received; `0` to start |
| `done` | `true` for the last chunk |
| `overwrite` | `true` to allow replacing an existing file |

The response contains `size`, the number of bytes received, and `path`, which
is `null` until completion. Chunks are written to a hidden temporary file,
published when the upload finishes. Without `overwrite`, publication uses a
hard link to avoid replacing a file created by another upload; the destination
filesystem must support hard links. With `overwrite`, it uses a rename.
Missing subdirectories are created.

The server accepts at most 64 MiB per request; the interface sends 32 MiB.
An oversized chunk receives `413` and its bytes are rolled back. An incorrect
`offset` receives `409`. A body-stream error rolls the chunk back to its starting
offset, allowing a retry. If rollback itself fails, or the server crashes, cancel
the upload before starting over with a new ID. Send chunks sequentially for
each upload ID.

`DELETE /api/fs/upload?dir=...&path=...&id=...` removes the temporary file.

### Download and preview

- `GET /api/fs/download?path=...` downloads a file or builds a ZIP for a
  directory. The ZIP skips symlinks, special files, and codeenv temporary
  files. It may include a report of skipped entries.
- `GET /api/fs/raw?path=...` displays an image or PDF. Other types receive
  `415`. SVGs receive a CSP policy that blocks scripts and prevents access
  to the interface's origin.

## Search

`GET /api/search/text?dir=...&q=...&regex=false&case=false` searches file
contents. `regex` enables regular expressions; `case` makes the search
case-sensitive.

Results stream as one JSON object per line (NDJSON). Example:

```json
{"file":"/home/dev/a.txt","matches":[{"line":3,"text":"hello","ranges":[[0,5]]}],"more":false}
{"done":true,"files":1,"matches":1,"truncated":false,"ms":12}
```

`line` starts at 1. `ranges` holds highlight positions within `text`, in UTF-16
units like JavaScript string indices. Long lines are shortened around the
first match. `more` means the file has additional matches; `truncated` signals
an overall result or time limit.

Search excludes `.git`, binary files, and files larger than 2 MiB. It normally
respects `.gitignore` and `.ignore`; see
[troubleshooting](troubleshooting.md#search-returns-unexpected-results) for
exceptions. Up to four content searches can run at once; further requests
receive `429`.

`GET /api/search/files?dir=...&q=...` searches file names and returns up to
50 results: `[{"path":"...","rel":"...","marks":[0,2]}]`.
`rel` is the path relative to the searched directory; `marks` gives UTF-16
positions to highlight.

## Terminals

| Request | Body or parameters | Result |
|---|---|---|
| `GET /api/terminals` | — | User's terminals |
| `POST /api/terminals` | `{"title":"...","cwd":"...","command":"..."}`; all fields optional | Created terminal |
| `PATCH /api/terminals/{id}` | `{"title":"..."}` | `{}` |
| `DELETE /api/terminals/{id}` | — | `{}`; requests terminal shutdown |
| `GET /api/terminals/{id}/ws` | `?cols=120&rows=32` | WebSocket connection |

A terminal has `id`, `title`, `owner`, `created`, `cwd`, `command`, `clients`,
`activity`, and `pid`. `created` and `activity` are Unix timestamps in seconds.
`command` is the foreground process name when the system can read it.

On creation, `cwd` can be absolute, relative to `root`, or start with `~/`.
An empty value means `root`. Unlike the explorer, the shell can access other
directories allowed by Unix permissions. `command` is sent to the shell after
startup. `max_terminals` limits creation per user; reaching the limit returns
`429`.

WebSocket binary frames carry terminal bytes. Text frames carry control
messages:

```json
{"type":"resize","cols":120,"rows":32}
```

The browser sends this message to resize. The server sends `{"type":"exit"}`
when the program exits. Closing the WebSocket leaves the terminal running.
See the [daemon protocol](protocol.md) for internal communication.

## Commands and ports

`PUT /api/buttons` replaces all personal commands. The body is a list of
objects with `name`, `command`, and optional `id`, `cwd`, and `color` fields.
Keep existing IDs when editing. The response contains the saved list with
missing IDs filled in.

`GET /api/ports` lists local ports allowed by the configuration. Each object
contains `port`, `process`, `pid`, `url`, `cmdline`, `cwd`, and `terminal`.
`url` is `null` when forwarding is not configured. `terminal` is `null` or
`{"id":"...","title":"..."}` when the originating terminal was found.
