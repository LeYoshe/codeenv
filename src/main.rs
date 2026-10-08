mod api;
mod assets;
mod auth;
mod config;
mod files;
mod ports;
mod procinfo;
mod proxy;
mod pty;
mod ptyd;
mod search;
mod store;
mod terminal;
mod transfer;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use clap::Parser;

use crate::auth::Auth;
use crate::config::{Config, HostTemplate};

#[derive(Parser)]
#[command(
    version,
    about = "Web terminals, file explorer and port forwarding for remote dev servers"
)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(
        short,
        long,
        env = "CODEENV_CONFIG",
        default_value = "/etc/codeenv/config.toml"
    )]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// The PTY daemon holding the terminals (started automatically).
    Ptyd {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long, default_value_t = 5000)]
        scrollback: usize,
    },
}

pub struct App {
    pub config: Config,
    pub auth: Auth,
    pub pty: pty::PtyClient,
    pub store: store::Store,
    /// Canonical explorer root.
    pub root: PathBuf,
    pub home: PathBuf,
    pub name: String,
    pub host_template: Option<HostTemplate>,
    pub proxy: proxy::HttpClient,
    pub own_port: u16,
}

impl App {
    /// Whether this request's Host header is one we serve: the UI (a
    /// configured host, or — in insecure mode — loopback) or a forwarded-port
    /// hostname. Returns false when a Host header is required but absent.
    fn host_allowed(&self, host: Option<&str>) -> bool {
        let insecure = matches!(self.auth, Auth::Insecure(_));
        // With no allowlist and real auth, any Host is accepted (the JWT, not
        // the Host, is what authenticates — so rebinding gains nothing).
        if self.config.hosts.is_empty() && !insecure {
            return true;
        }
        let Some(host) = host else { return false };
        let name = config::host_name(host).to_ascii_lowercase();
        if self.config.hosts.contains(&name) {
            return true;
        }
        if insecure && config::is_loopback_host(host) {
            return true;
        }
        self.host_template
            .as_ref()
            .and_then(|t| t.match_host(host))
            .is_some()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "codeenv=info".into()),
        )
        .init();
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("installing rustls crypto provider"))?;

    let cli = Cli::parse();
    if let Some(Command::Ptyd { socket, scrollback }) = cli.command {
        return ptyd::run(&socket, scrollback).await;
    }
    let config = Config::load(&cli.config)?;

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("$HOME is not set")?;
    let root = config.root.clone().unwrap_or_else(|| home.clone());
    let root = root
        .canonicalize()
        .with_context(|| format!("root {}", root.display()))?;
    let data_dir = config
        .data_dir
        .clone()
        .unwrap_or_else(|| home.join(".local/share/codeenv"));
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating {}", data_dir.display()))?;
    // Holds the ptyd socket, logs and per-user data (button commands may carry
    // secrets): keep it private to this user.
    #[cfg(unix)]
    std::fs::set_permissions(
        &data_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .with_context(|| format!("securing {}", data_dir.display()))?;
    let shell = config
        .shell
        .clone()
        .or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "/bin/bash".into());
    let name = config.name.clone().unwrap_or_else(hostname);
    let listen_addr: SocketAddr = config
        .listen
        .parse()
        .with_context(|| format!("listen {:?}", config.listen))?;

    let auth = Auth::new(&config.auth)?;
    match &auth {
        Auth::Access(verifier) => verifier
            .prefetch()
            .await
            .context("fetching Cloudflare Access keys")?,
        Auth::Insecure(user) => {
            // Insecure mode trusts the network, so it must stay on the machine:
            // bound to loopback, and (DNS rebinding) only answering local Hosts.
            if !listen_addr.ip().is_loopback() {
                anyhow::bail!(
                    "auth.mode = \"insecure\" may only listen on a loopback address (got {listen_addr}); \
                     it has no authentication and must never be exposed"
                );
            }
            tracing::warn!(
                "auth.mode = \"insecure\": every request is accepted as {user}. Never expose this through a tunnel."
            );
        }
    }

    let app = Arc::new(App {
        pty: pty::PtyClient::start(&data_dir, shell, config.scrollback).await?,
        store: store::Store::new(&data_dir)?,
        host_template: config
            .ports
            .host_template
            .as_deref()
            .map(HostTemplate::parse)
            .transpose()?,
        proxy: proxy::client(),
        own_port: listen_addr.port(),
        auth,
        root,
        home,
        name,
        config,
    });

    let router = router(app.clone());

    // Refresh the Access signing keys hourly, so retired keys stop being
    // accepted even if no request with an unknown kid arrives.
    if matches!(app.auth, Auth::Access(_)) {
        let app = app.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
            tick.tick().await; // the immediate first tick (already prefetched)
            loop {
                tick.tick().await;
                app.auth.refresh_now().await;
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .with_context(|| format!("binding {listen_addr}"))?;
    tracing::info!(
        "codeenv \"{}\" listening on http://{listen_addr} (root {})",
        app.name,
        app.root.display()
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// Rejects requests whose Host header we do not serve (DNS rebinding, a
/// tunnel catch-all pointing stray hostnames at us).
async fn check_host(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok());
    if app.host_allowed(host) {
        return next.run(req).await;
    }
    tracing::warn!("rejected request for Host {:?}", host.unwrap_or("<none>"));
    (
        axum::http::StatusCode::MISDIRECTED_REQUEST,
        "unknown host\n",
    )
        .into_response()
}

/// Requests for `p{port}-….` hostnames go to the local port, everything else
/// to the UI.
async fn dispatch_forwarded_host(
    State(app): State<Arc<App>>,
    req: Request,
    next: Next,
) -> Response {
    if let Some(template) = &app.host_template {
        let port = req
            .headers()
            .get(axum::http::header::HOST)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| template.match_host(h));
        if let Some(port) = port {
            return proxy::forward(app.clone(), req, port).await;
        }
    }
    next.run(req).await
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("installing SIGTERM handler");
        tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
    tracing::info!("shutting down (terminals keep running in ptyd)");
    // Open WebSockets never finish on their own; don't let them hold the exit.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        std::process::exit(0);
    });
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buf is valid for writes of its length.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    if ok && end > 0 {
        String::from_utf8_lossy(&buf[..end]).into_owned()
    } else {
        "codeenv".into()
    }
}

fn router(app: Arc<App>) -> Router {
    Router::new()
        .merge(api::routes())
        .merge(assets::routes())
        .layer(middleware::from_fn_with_state(
            app.clone(),
            dispatch_forwarded_host,
        ))
        .layer(middleware::from_fn_with_state(
            app.clone(),
            auth::require_user,
        ))
        // Outermost: reject unexpected Host headers before anything else runs.
        .layer(middleware::from_fn_with_state(app.clone(), check_host))
        .with_state(app)
}

#[cfg(test)]
mod integration_tests;
