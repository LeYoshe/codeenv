# How codeenv talks to terminals

Shells run in a separate process, `codeenv ptyd`, which stays alive when the
web server restarts. codeenv starts it automatically and communicates through
the Unix socket `data_dir/ptyd.sock`.

This document covers internal communication. To connect a browser or write a
web client, see the [HTTP and WebSocket reference](api.md#terminals).
The implementation is in `src/ptyd.rs`, `src/pty.rs`, and `src/terminal.rs`.

## Message format

Each message starts with a four-byte length, followed by a one-byte type and
the payload:

```text
length (u32 big-endian) | type (u8) | payload
```

The length includes the type and payload, but excludes the four length bytes.
The reader accepts lengths from 1 byte to 16 MiB. Type `1` is JSON; type `2`
is terminal bytes.

A connection normally carries one request and its response. `attach` keeps
it open for keyboard input and terminal output. A connection ending during
a message is an error, not an empty message.

## Who can connect

The socket is private, with permissions `0600`. The daemon also checks that
the client's Unix user matches its own.

Any process under that account can control terminals. codeenv checks ownership
by email in the web API; the daemon does not verify Cloudflare Access identity.

## Requests

JSON requests have an `op` field:

| `op` | Other fields | Action |
|---|---|---|
| `hello` | — | Read protocol and binary versions |
| `list` | — | List all terminals |
| `create` | See example below | Create a terminal |
| `kill` | `id` | Request terminal shutdown |
| `rename` | `id`, `title` | Change its title |
| `attach` | `id`, `cols`, `rows` | Receive its state, then new output |
| `resize` | `cols`, `rows` | Resize over an attached connection |

Example creation request:

```json
{
  "op": "create",
  "owner": "dev@example.com",
  "title": "Project",
  "cwd": "/home/dev/project",
  "shell": "/bin/bash",
  "env": [["TERM", "xterm-256color"]],
  "command": "pwd",
  "cols": 120,
  "rows": 32
}
```

`command` is optional. The shell receives the variables in `env` and a selected
set from the daemon, including `HOME`, `PATH`, and locale settings. The most
recent attachment or resize request sets the size shared by all clients of
that terminal.

## Responses

A successful response contains `"ok":true` and any relevant data:

```json
{"ok":true,"protocol":1,"version":"0.1.0"}
```

`create` returns `id`; `list` returns `terminals`. An error contains
`"ok":false` and an `error` field.

## Attach to an existing terminal

After `attach`, the daemon:

1. Responds with `{"ok":true}`.
2. Sends terminal state in binary frames of at most 256 KiB: scrollback,
   screen, cursor, and modes tracked by the vt100 emulator.
3. Forwards new output in binary frames. Meanwhile, the client can send
   typed text, keys, and `resize` requests.
4. Sends `{"ok":true,"event":"exit"}` when the program exits.

The client resets its display terminal before replaying the received state.
A client that reads too slowly is disconnected. Reconnecting retrieves the
current state. Disconnecting a client does not stop the shell.

In the WebSocket bridge, each read must stay alive until the whole message
arrives. Cancelling a partial read when a key arrives would lose the position
in the stream.

## Stop programs

`kill` sends SIGHUP to the shell and foreground process group. After three
seconds, the daemon attempts SIGKILL if the watcher thread has not yet reaped
the shell. This does not supervise every descendant: a detached program may
keep running.

When terminal reading ends and the shell has been reaped, the daemon removes
the session and notifies clients. Session state is held in memory and is lost
when the daemon stops or the machine reboots.

## Versions and updates

`PROTOCOL`, currently `1`, is returned by `hello`. codeenv refuses to work
with a daemon reporting another version. Incompatible changes to the message
format must increment this value.

A compatible web server update leaves the old daemon running. Daemon changes
take effect only after restarting it. This closes its terminals: finish
running work before following the
[restart procedure](troubleshooting.md#restart-the-daemon).
