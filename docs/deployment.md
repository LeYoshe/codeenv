# Deploy codeenv on a VPS

codeenv listens on a local address. `cloudflared`, running on the same VPS,
forwards browser connections. Cloudflare Access controls who can connect.

The examples use the Unix account `codeenv`, domain `example.com`, and machine
name `vps1`. Replace them with your own values.

## Prepare the machine

You need a Linux VPS, a recent Rust toolchain, `cloudflared`, and a domain
configured in Cloudflare. The Unix account running codeenv must be able to
read and modify your projects.

Anyone allowed through Access can open a shell under this account. Use a
dedicated account without passwordless sudo. The examples assume an existing
`codeenv` account with a home directory at `/home/codeenv`; create it and grant
it access to your projects before starting the service.

## Build and install

Building from source needs a C/C++ compiler, Make, and CMake. On Debian or
Ubuntu, install `build-essential` and `cmake`. Prebuilt binaries do not need
these tools.

From the repository, on the VPS or a compatible Linux machine:

```sh
cargo build --locked --release
sudo install -m 0755 target/release/codeenv /usr/local/bin/codeenv
sudo mkdir -p /etc/codeenv
sudo cp config.example.toml /etc/codeenv/config.toml
sudo chmod 0644 /etc/codeenv/config.toml
```

Edit `/etc/codeenv/config.toml`. A minimal configuration looks like this:

```toml
listen = "127.0.0.1:7681"
name = "vps1"
root = "/home/codeenv"
hosts = ["vps1.example.com"]

[auth]
mode = "cloudflare_access"
team_domain = "myteam"
audiences = ["REPLACE_WITH_ACCESS_AUD_TAG"]
```

`root` must be an existing directory. `hosts` lists the interface's domain
names. Optional settings are documented in
[config.example.toml](../config.example.toml).

By default, data is stored in `~/.local/share/codeenv` under the service
account. This directory holds personal commands, the daemon socket, and its
log. codeenv restricts access to this Unix account.

## Configure Cloudflare access

Authenticate `cloudflared` on the VPS:

```sh
cloudflared tunnel login
```

If the VPS has no browser, open the URL printed by the command in your own
computer's browser. Leave the command running on the VPS while you sign in;
it downloads the account certificate when authentication finishes.

Create a tunnel named `vps1`, then a DNS record for the interface:

```sh
cloudflared tunnel create vps1
cloudflared tunnel route dns vps1 vps1.example.com
```

This guide uses a locally managed tunnel. See Cloudflare's
[local tunnel guide](https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/do-more-with-tunnels/local-management/create-local-tunnel/)
for the full setup procedure.

Copy [deploy/cloudflared.yml](../deploy/cloudflared.yml) to
`/etc/cloudflared/config.yml`. Replace the tunnel ID and set the actual path
to the credentials file created by `cloudflared`. The account running
`cloudflared` must be able to read it.

In Cloudflare Zero Trust, create a **Self-hosted** Access application for
`vps1.example.com`. Add an **Allow** policy restricted to the intended users.
Copy these values into the codeenv configuration:

- The team name into `auth.team_domain`.
- The **Application Audience (AUD)** tag into `auth.audiences`.

codeenv verifies the token sent by Access. Direct requests without a token
are rejected. Service tokens without an email address are not supported.

Keep the interface hostname in `hosts`, as shown in the configuration above.
This rejects unrelated hostnames with `421 Misdirected Request` before
authentication, including names sent to the tunnel by a wildcard rule.
Forwarded-port hostnames are accepted through `ports.host_template`, so they
do not belong in `hosts`.

Install and start the tunnel for your `cloudflared` setup. To use its provided
service:

```sh
sudo cloudflared service install
sudo systemctl enable --now cloudflared
```

## Start codeenv with systemd

```sh
sudo cp deploy/codeenv.service /etc/systemd/system/codeenv.service
```

Edit `User=` and `Group=` in this file. Keep `KillMode=process`: it lets
terminals survive web server stops and restarts. `UMask=0077` restricts new
files to the service account.

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now codeenv
journalctl -u codeenv -f
```

Open `https://vps1.example.com`, sign in through Access, and open a terminal.
If startup fails, see [troubleshooting](troubleshooting.md).

## Access apps started in terminals

To open a development server on port 5173, add this to codeenv's configuration:

```toml
[ports]
host_template = "p{port}-vps1.example.com"
```

The port will be available at `https://p5173-vps1.example.com`. You also need:

1. A DNS record pointing to the tunnel. For this specific port:
   `cloudflared tunnel route dns vps1 p5173-vps1.example.com`.
2. A `cloudflared` rule forwarding that hostname to codeenv. The template
   includes a `*.example.com` rule; remove it if unused.
3. An Access application protecting that hostname. If it is separate from the
   interface's application, add its AUD tag to `auth.audiences` too.
4. A TLS certificate covering the chosen hostname.

For several ports, a wildcard DNS record can replace individual records. It
points to one tunnel: with multiple VPS instances, use distinct names and
check each route. Reserve these names for codeenv to avoid conflicts with
other sites.

DNS, TLS certificates, Access, and `cloudflared` have different wildcard rules:

- **DNS:** use `*.example.com`. The asterisk must be a whole leftmost label;
  `*-vps1.example.com` does not match `p5173-vps1.example.com`. See
  [wildcard DNS records](https://developers.cloudflare.com/dns/manage-dns-records/reference/wildcard-dns-records/).
- **TLS:** with Cloudflare's full DNS setup, Universal SSL covers the apex and
  first-level subdomains, including `p5173-vps1.example.com`. A deeper name
  such as `p5173.vps1.example.com` needs additional certificate coverage, for
  example an advanced or custom certificate. CNAME setups have different
  coverage rules. See [Universal SSL limitations](https://developers.cloudflare.com/ssl/edge-certificates/universal-ssl/limitations/).
- **Access:** `*-vps1.example.com` matches forwarded ports, but also names such
  as `admin-vps1.example.com`. codeenv's `host_template` restricts which of
  those names it serves as ports. Give a separate Access application its own
  AUD tag in `auth.audiences`. See [Access wildcard matching](https://developers.cloudflare.com/cloudflare-one/access-controls/policies/app-paths/).
- **cloudflared:** use `*.example.com` in the ingress rule. Do not use Access's
  partial wildcard syntax here. Keep the final `http_status:404` rule for
  unmatched requests.

Validate the configuration and check which rule handles a forwarded port
before restarting the tunnel:

```sh
cloudflared tunnel ingress validate
cloudflared tunnel ingress rule https://p5173-vps1.example.com/
```

After adding wildcard DNS, compare the failing name with a fresh random name
at the same level, such as `probe-<random-hex>.example.com`. Query both the
configured resolver and the authoritative nameserver. A previously queried
name returning `NXDOMAIN`, even from the authoritative service, is not enough
to conclude that the wildcard is broken.

If the fresh name resolves but the old one does not, check for explicit DNS
records or delegations that override the wildcard, then allow earlier answers
to expire and retry. A fresh name is a useful comparison, not a guarantee that
every hostname will resolve. Test DNS separately: this probe hostname need not
be accepted by codeenv or Access.

Without `host_template`, forwarding is disabled. Apps must listen on
`localhost` or all interfaces. Each forwarded port gets its own subdomain,
separate from the codeenv interface.

## Update

Build the new version, then replace the binary by renaming it. This avoids
overwriting the executable file of a daemon that is still running:

```sh
cargo build --locked --release
sudo install -m 0755 target/release/codeenv /usr/local/bin/codeenv.new
sudo mv /usr/local/bin/codeenv.new /usr/local/bin/codeenv
sudo systemctl restart codeenv
```

Reload the browser page. Terminals keep running in the old daemon. If the new
version changes the protocol incompatibly, codeenv refuses to start and
reports the mismatch. Stop the daemon, which closes its terminals, then
restart the service. See [protocol versions](protocol.md#versions-and-updates).

Back up `data_dir/users` to preserve personal commands. Back up projects
separately. Terminal state is held in memory and does not survive a VPS reboot.
