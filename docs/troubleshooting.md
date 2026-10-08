# Troubleshooting

Start with the logs:

```sh
journalctl -u codeenv -n 100 --no-pager
```

The terminal daemon writes to `data_dir/ptyd.log`, which defaults to
`~/.local/share/codeenv/ptyd.log` under the service account. For more detail,
set `RUST_LOG=codeenv=debug` when starting codeenv.

## The server will not start

| Message or symptom | What to check |
|---|---|
| `may only listen on a loopback address` | `insecure` mode requires a local address such as `127.0.0.1`. Use `cloudflare_access` in production. |
| `fetching Cloudflare Access keys` | Check `team_domain` and network access from the VPS to `https://<team>.cloudflareaccess.com/cdn-cgi/access/certs`. |
| `root` | The configured folder must exist and be accessible to the service account. |
| `data_dir` | The service account must be able to write to this directory. |
| `binding` | Check the address and whether another process already uses the port. |
| `ptyd did not start` | Read `ptyd.log`. Check permissions and the length of the `data_dir` path. |
| `speaks protocol` | The running daemon is incompatible with the new server. See the restart procedure below. |

Restart codeenv after configuration changes. `scrollback` is applied when the
daemon starts: changing it also requires restarting the daemon, which closes
existing terminals.

## Access is denied

**403 with `Cloudflare Access authentication required`**: the request does not
contain an accepted Access token. Use the protected domain, sign in to Access
again, and check `team_domain` and `audiences`.

**421 with `unknown host`**: the requested hostname is not in `hosts` and does
not match the port forwarding template. Check the tunnel rules too.

**403 with `cross-origin request`**: the API call comes from another origin.
Scripts that modify data must send an `Origin` header such as
`Origin: https://vps1.example.com`, matching the request's hostname and port.
See the [API reference](api.md) for a complete example.

## A terminal is missing or unresponsive

Closing the browser or restarting codeenv leaves terminals running. Closing
a terminal tab **inside codeenv** stops it. Rebooting the VPS or stopping the
daemon also closes terminals.

Check the terminal panel: the tab may simply be hidden. Then check that the
service uses the right `data_dir` and retains `KillMode=process`:

```sh
pgrep -af 'codeenv ptyd'
systemctl cat codeenv
```

Repeated reconnections may be caused by the network, an expired Access
session, or a daemon problem. Reload the page and compare the service and
daemon logs.

## The terminal has the wrong size

Reload the page, then resize the window. If this persists, check the browser
console for errors and ensure that a custom Cloudflare rule is not caching `/`.

## The browser still intercepts some keys

Browsers may reserve `Ctrl+W`, `Ctrl+T`, or `Ctrl+N`. In supported browsers,
the keyboard button enables fullscreen and keyboard lock. Hold `Escape` to
leave this mode.

## A forwarded app will not open

Check these in order:

1. Does the app respond on `localhost:<port>` from the VPS?
2. Is `ports.host_template` configured? The port must not be listed in
   `ports.deny` or be codeenv's own port.
3. Does the port's DNS record point to the right tunnel?
4. Do the tunnel and Access cover this hostname? Is its certificate valid?

By default, codeenv sends `Host: localhost:<port>` to the app. If it expects
its public hostname, try `ports.rewrite_host = false`.

## The tunnel credentials file is missing

For an existing locally managed tunnel, recover the credentials without
recreating the tunnel or its DNS records. With the account certificate from
`cloudflared tunnel login`, run this as the authenticated Unix user. Replace
`<TUNNEL-UUID>` with the existing tunnel's ID:

```sh
umask 077
cloudflared tunnel token --cred-file ./recovered-tunnel.json <TUNNEL-UUID>
sudo install -m 0600 ./recovered-tunnel.json /etc/cloudflared/<TUNNEL-UUID>.json
```

The destination above assumes `cloudflared` runs as root. If it uses a dedicated
account, set the file's owner to that account. Point `credentials-file` in
`/etc/cloudflared/config.yml` at the restored file, then restart `cloudflared`.
Remove the temporary `recovered-tunnel.json` copy once recovery is complete.
This command supports tunnels created with cloudflared 2022.3.0 or later.

If you use an API token instead of an account certificate, the
[Get a Cloudflare Tunnel token endpoint](https://developers.cloudflare.com/api/resources/zero_trust/subresources/tunnels/subresources/cloudflared/subresources/token/methods/get/)
also retrieves the credentials:

```text
GET /accounts/{account_id}/cfd_tunnel/{tunnel_id}/token
Authorization: Bearer <API_TOKEN>
```

Use a token authorized for the account with `Cloudflare Tunnel Write` (called
Cloudflare Tunnel: Edit in the dashboard), or another permission accepted by
that endpoint. The response's `result` is a base64-encoded JSON object. Decode
it and map the fields into the credentials JSON:

| Token field | Credentials field |
|---|---|
| `a` | `AccountTag` |
| `t` | `TunnelID` |
| `s` | `TunnelSecret` |
| `e`, if present | `Endpoint` |

Keep `s` as the base64 string stored in the decoded JSON; do not decode it
again. Save the resulting credentials to `/etc/cloudflared/<TUNNEL-UUID>.json`
with mode `0600` and the service account as owner. These values allow a
connector to run the tunnel: keep both the token and the file out of Git and
issue reports. This restores credentials; it does not convert a remotely
managed tunnel into a locally managed one.

## A file will not open or save

The editor accepts UTF-8 text up to 5 MiB. Download larger or binary files.
Read-only files require a permissions change on the VPS.

If the editor reports that the file changed on disk, reload it to see that
version or confirm replacing it with your content. Reloading discards
unsaved changes.

The explorer rejects paths outside `root`, including symlinks leading outside
it. Special files such as FIFOs cannot be opened in the editor.

## Search returns unexpected results

Check the folder shown below the search field, then the case and regular
expression options. Content search excludes binary files and files larger
than 2 MiB.

If codeenv finds an unusual `.gitignore` or `.ignore` in the folder (a symlink,
a special file, or a file over 1 MiB), it disables ignore rules for that
search. The log identifies the file. Replace it with a regular text file to
restore ignore rules.

A limited search may miss results. Narrow the folder or make the query more
specific. Connection errors and interrupted searches are shown below the
search field.

## An upload fails

Check disk space and destination folder permissions. A conflict can mean that
a file with the same name already exists or that the server did not receive
the expected chunk.

The interface lets you cancel an upload. An interrupted upload may leave a
hidden file named `.name.ce-upload-id` in the destination folder. Before
deleting it manually, make sure the upload has finished or been abandoned.

## Restart the daemon

This closes all affected terminals. Finish running commands first. Under the
service's Unix account:

```sh
sudo systemctl stop codeenv
pgrep -af 'codeenv ptyd'
kill <DAEMON_PID>
sudo systemctl start codeenv
```

Choose the PID whose `--socket` argument matches your `data_dir`, especially
if several instances run on the machine. Do not delete `data_dir` to fix a
connection problem: it also holds personal commands.
