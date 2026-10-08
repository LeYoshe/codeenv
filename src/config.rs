use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Address the HTTP server binds to. cloudflared connects here.
    #[serde(default = "default_listen")]
    pub listen: String,
    /// Display name of this machine in the UI (defaults to the hostname).
    pub name: Option<String>,
    /// Root of the file explorer (defaults to $HOME).
    pub root: Option<PathBuf>,
    /// Where per-user data (buttons, …) is stored.
    pub data_dir: Option<PathBuf>,
    /// Shell started in new terminals (defaults to $SHELL, then /bin/bash).
    pub shell: Option<String>,
    /// Allowed UI hostnames. Empty accepts any host in Access mode.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Lines of history kept per terminal and replayed to a browser that
    /// (re)attaches.
    #[serde(default = "default_scrollback")]
    pub scrollback: usize,
    /// Maximum number of live terminals per user.
    #[serde(default = "default_max_terminals")]
    pub max_terminals: usize,
    pub auth: AuthConfig,
    #[serde(default)]
    pub ports: PortsConfig,
    /// Buttons visible to every user (read-only in the UI).
    #[serde(default)]
    pub buttons: Vec<Button>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthConfig {
    /// Every request must carry a valid Cloudflare Access JWT
    /// (`Cf-Access-Jwt-Assertion`), signed by the team's keys and issued for
    /// one of `audiences` (the "Application Audience (AUD) Tag").
    CloudflareAccess {
        /// e.g. "myteam" or "myteam.cloudflareaccess.com"
        team_domain: String,
        audiences: Vec<String>,
    },
    /// No authentication: every request is treated as `user`. Only for local
    /// development — never expose this through a tunnel.
    Insecure { user: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortsConfig {
    /// Public hostname per port, e.g. "p{port}-vps1.example.com".
    /// Forwarding is disabled when absent. Requires matching DNS and ingress.
    pub host_template: Option<String>,
    /// Send `Host: localhost:{port}` upstream instead of the public host.
    /// Dev servers (Vite, webpack, …) reject unknown hosts by default.
    #[serde(default = "default_rewrite_host")]
    pub rewrite_host: bool,
    /// Ports that may never be forwarded.
    #[serde(default)]
    pub deny: Vec<u16>,
}

impl Default for PortsConfig {
    fn default() -> Self {
        Self {
            host_template: None,
            rewrite_host: true,
            deny: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Button {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub command: String,
    /// Working directory. Absolute, `~/…`, or relative to the explorer root.
    /// Empty: the folder currently selected in the explorer.
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub color: String,
}

fn default_listen() -> String {
    "127.0.0.1:7681".into()
}
fn default_scrollback() -> usize {
    5000
}
fn default_max_terminals() -> usize {
    50
}
fn default_rewrite_host() -> bool {
    true
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let mut config: Config =
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
        if let Some(template) = &config.ports.host_template {
            HostTemplate::parse(template)?;
        }
        // Normalize the host allowlist: lowercase, no port, no scheme.
        for host in &mut config.hosts {
            *host = host_name(
                host.trim()
                    .trim_start_matches("https://")
                    .trim_end_matches('/'),
            )
            .to_ascii_lowercase();
            if host.is_empty() || host.contains('/') {
                bail!("hosts: {host:?} is not a hostname");
            }
        }
        for (i, button) in config.buttons.iter_mut().enumerate() {
            if button.name.trim().is_empty() || button.command.trim().is_empty() {
                bail!("buttons[{i}]: name and command are required");
            }
            if button.id.is_empty() {
                button.id = format!("global-{i}");
            }
        }
        Ok(config)
    }
}

/// The hostname part of a Host header, without the port.
/// `[::1]:8080` → `[::1]`, `VPS1.example.com:443` → `VPS1.example.com`.
pub fn host_name(host: &str) -> &str {
    let host = host.trim();
    // IPv6 literal in brackets: the colons inside are not a port separator.
    if host.starts_with('[') {
        return host.find(']').map(|end| &host[..=end]).unwrap_or(host);
    }
    match host.rsplit_once(':') {
        Some((name, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            name
        }
        _ => host,
    }
}

/// Whether `host` denotes the local machine (loopback or "localhost").
pub fn is_loopback_host(host: &str) -> bool {
    let name = host_name(host);
    if name.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let ip = name
        .strip_prefix('[')
        .and_then(|name| name.strip_suffix(']'))
        .unwrap_or(name);
    ip.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// A hostname of the form `<prefix>{port}<suffix>`.
#[derive(Debug, Clone)]
pub struct HostTemplate {
    prefix: String,
    suffix: String,
}

impl HostTemplate {
    pub fn parse(template: &str) -> Result<Self> {
        let template = template.trim().to_ascii_lowercase();
        let Some((prefix, suffix)) = template.split_once("{port}") else {
            bail!("ports.host_template must contain {{port}}: {template:?}");
        };
        if suffix.contains("{port}") {
            bail!("ports.host_template must contain {{port}} only once: {template:?}");
        }
        if !suffix.contains('.') {
            bail!("ports.host_template must end with a domain after {{port}}: {template:?}");
        }
        if template.contains("://") || template.contains('/') || template.contains(':') {
            bail!("ports.host_template is a hostname, not a URL: {template:?}");
        }
        Ok(Self {
            prefix: prefix.into(),
            suffix: suffix.into(),
        })
    }

    /// Returns the port if `host` (as sent in the Host header) matches.
    pub fn match_host(&self, host: &str) -> Option<u16> {
        let host = host_name(host).to_ascii_lowercase();
        let middle = host
            .strip_prefix(&self.prefix)?
            .strip_suffix(&self.suffix)?;
        if middle.is_empty() || !middle.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        middle.parse().ok().filter(|p| *p != 0)
    }

    pub fn template(&self) -> String {
        format!("{}{{port}}{}", self.prefix, self.suffix)
    }

    pub fn host_for(&self, port: u16) -> String {
        format!("{}{}{}", self.prefix, port, self.suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_template() {
        let template = HostTemplate::parse("p{port}-vps1.example.com").unwrap();
        assert_eq!(template.match_host("p5173-vps1.example.com"), Some(5173));
        assert_eq!(
            template.match_host("P5173-VPS1.example.com:443"),
            Some(5173)
        );
        assert_eq!(template.match_host("p-vps1.example.com"), None);
        assert_eq!(template.match_host("p99999-vps1.example.com"), None);
        assert_eq!(template.match_host("px1-vps1.example.com"), None);
        assert_eq!(template.match_host("vps1.example.com"), None);
        assert_eq!(template.host_for(3000), "p3000-vps1.example.com");
        assert!(HostTemplate::parse("example.com").is_err());
        assert!(HostTemplate::parse("https://{port}.example.com").is_err());
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_name("vps1.example.com:443"), "vps1.example.com");
        assert_eq!(host_name("VPS1.example.com"), "VPS1.example.com");
        assert_eq!(host_name("[::1]:8080"), "[::1]");
        assert_eq!(host_name("[::1]"), "[::1]");
        assert_eq!(host_name("[::1"), "[::1");
        assert_eq!(host_name("127.0.0.1:7681"), "127.0.0.1");
        assert!(is_loopback_host("localhost:7681"));
        assert!(is_loopback_host("127.0.0.1:7681"));
        assert!(is_loopback_host("[::1]:7681"));
        assert!(!is_loopback_host("evil.example.com:7681"));
        assert!(!is_loopback_host("127evil.com"));
        assert!(!is_loopback_host("127.example.com"));
        assert!(!is_loopback_host("127.0.0.1.example.com"));
        assert!(!is_loopback_host("127.999.0.1"));
        assert!(is_loopback_host("127.12.34.56:7681"));
    }
}
