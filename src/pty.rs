//! codeenv's side of the PTY daemon: starts it if needed and talks to it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::net::UnixStream;

use crate::ptyd::{self, CreateRequest, PROTOCOL, Request, Response, TerminalInfo};

pub struct PtyClient {
    socket: PathBuf,
    shell: String,
}

/// sun_path is 104 bytes on macOS, 108 on Linux.
const MAX_SOCKET_PATH: usize = 100;

impl PtyClient {
    #[cfg(test)]
    pub(crate) fn for_test(socket: PathBuf) -> Self {
        Self {
            socket,
            shell: "/bin/bash".into(),
        }
    }

    /// Connects to the daemon serving `data_dir`, starting it if none runs.
    pub async fn start(data_dir: &Path, shell: String, scrollback: usize) -> Result<Self> {
        let socket = data_dir.join("ptyd.sock");
        if socket.as_os_str().len() > MAX_SOCKET_PATH {
            bail!(
                "{} is too long for a Unix socket; use a shorter data_dir",
                socket.display()
            );
        }
        let client = Self { socket, shell };
        let hello = match client.request(&Request::Hello).await {
            Ok(response) => response,
            Err(_) => {
                spawn_daemon(&client.socket, data_dir, scrollback)?;
                client.wait_ready().await?
            }
        };
        let protocol = hello.protocol.unwrap_or(0);
        if protocol != PROTOCOL {
            bail!(
                "the running ptyd (version {}) speaks protocol {protocol}, this codeenv needs {PROTOCOL}. \
                 Stop it to upgrade — this ends all terminals: pkill -f 'codeenv ptyd'",
                hello.version.unwrap_or_default()
            );
        }
        tracing::info!(
            "ptyd {} on {}",
            hello.version.unwrap_or_default(),
            client.socket.display()
        );
        Ok(client)
    }

    async fn wait_ready(&self) -> Result<Response> {
        let mut last_error = None;
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            match self.request(&Request::Hello).await {
                Ok(response) => return Ok(response),
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("timeout")))
            .context("ptyd did not start (see ptyd.log in data_dir)")
    }

    pub fn shell(&self) -> &str {
        &self.shell
    }

    async fn connect(&self) -> Result<UnixStream> {
        UnixStream::connect(&self.socket)
            .await
            .with_context(|| format!("connecting to {}", self.socket.display()))
    }

    async fn request(&self, req: &Request) -> Result<Response> {
        let mut stream = self.connect().await?;
        ptyd::write_json(&mut stream, req).await?;
        let (kind, payload) = ptyd::read_frame(&mut stream)
            .await?
            .ok_or_else(|| anyhow!("ptyd closed the connection"))?;
        if kind != ptyd::KIND_JSON {
            bail!("unexpected frame from ptyd");
        }
        let response: Response = serde_json::from_slice(&payload)?;
        if !response.ok {
            bail!("{}", response.error.unwrap_or_else(|| "ptyd error".into()));
        }
        Ok(response)
    }

    pub async fn list_for(&self, owner: &str) -> Result<Vec<TerminalInfo>> {
        let mut terminals: Vec<TerminalInfo> = self
            .request(&Request::List)
            .await?
            .terminals
            .unwrap_or_default()
            .into_iter()
            .filter(|terminal| terminal.owner == owner)
            .collect();
        terminals.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.id.cmp(&b.id)));
        Ok(terminals)
    }

    /// The terminal if it exists and belongs to `owner`.
    pub async fn get(&self, owner: &str, id: &str) -> Result<Option<TerminalInfo>> {
        Ok(self
            .list_for(owner)
            .await?
            .into_iter()
            .find(|terminal| terminal.id == id))
    }

    pub async fn create(
        &self,
        owner: &str,
        title: &str,
        cwd: &Path,
        command: Option<&str>,
    ) -> Result<String> {
        let env = vec![
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
            ("CODEENV".into(), "1".into()),
            ("CODEENV_USER".into(), owner.into()),
        ];
        let response = self
            .request(&Request::Create(CreateRequest {
                owner: owner.into(),
                title: clean_title(title),
                cwd: cwd.into(),
                shell: self.shell.clone(),
                env,
                command: command.map(String::from),
                cols: 120,
                rows: 32,
            }))
            .await?;
        response.id.ok_or_else(|| anyhow!("ptyd returned no id"))
    }

    pub async fn rename(&self, id: &str, title: &str) -> Result<()> {
        self.request(&Request::Rename {
            id: id.into(),
            title: clean_title(title),
        })
        .await
        .map(|_| ())
    }

    pub async fn kill(&self, id: &str) -> Result<()> {
        self.request(&Request::Kill { id: id.into() })
            .await
            .map(|_| ())
    }

    /// An attached stream: first frame is the snapshot, then live output.
    pub async fn attach(&self, id: &str, cols: u16, rows: u16) -> Result<UnixStream> {
        let mut stream = self.connect().await?;
        ptyd::write_json(
            &mut stream,
            &Request::Attach {
                id: id.into(),
                cols,
                rows,
            },
        )
        .await?;
        let (kind, payload) = ptyd::read_frame(&mut stream)
            .await?
            .ok_or_else(|| anyhow!("ptyd closed the connection"))?;
        let response: Response = if kind == ptyd::KIND_JSON {
            serde_json::from_slice(&payload)?
        } else {
            bail!("unexpected frame")
        };
        if !response.ok {
            bail!("{}", response.error.unwrap_or_default());
        }
        Ok(stream)
    }
}

/// Starts `codeenv ptyd` in its own session so that it survives codeenv
/// (restarts, upgrades, crashes) and is not hit by signals sent to it.
fn spawn_daemon(socket: &Path, data_dir: &Path, scrollback: usize) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().context("locating the codeenv binary")?;
    let log_path = data_dir.join("ptyd.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("ptyd")
        .arg("--socket")
        .arg(socket)
        .arg("--scrollback")
        .arg(scrollback.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn().context("starting ptyd")?;
    tracing::info!("started ptyd (pid {})", child.id());
    // Reap it if it exits early (e.g. another daemon won the lock); a
    // long-lived daemon is simply never waited for.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// Titles are shown in the UI; keep them short and printable.
fn clean_title(title: &str) -> String {
    let title: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
    let title = title.trim();
    if title.is_empty() {
        "terminal".into()
    } else {
        title.into()
    }
}
