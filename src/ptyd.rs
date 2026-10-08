//! Owns the shells independently of the web server.
//!
//! A Unix socket carries requests and terminal bytes. A vt100 parser keeps
//! screen state for reconnecting clients. See docs/protocol.md for the format.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

/// Bumped on any incompatible change of the messages below.
pub const PROTOCOL: u32 = 1;

pub const KIND_JSON: u8 = 1;
pub const KIND_DATA: u8 = 2;
const MAX_FRAME: usize = 16 << 20;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Hello,
    List,
    Create(CreateRequest),
    Kill {
        id: String,
    },
    Rename {
        id: String,
        title: String,
    },
    Attach {
        id: String,
        cols: u16,
        rows: u16,
    },
    /// Only valid on an attached stream.
    Resize {
        cols: u16,
        rows: u16,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRequest {
    pub owner: String,
    pub title: String,
    pub cwd: PathBuf,
    pub shell: String,
    pub env: Vec<(String, String)>,
    /// Typed into the shell once it is started.
    pub command: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalInfo {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub owner: String,
    pub created: u64,
    /// Working directory of the foreground process.
    pub cwd: String,
    /// Name of the foreground process (e.g. "zsh", "node", "vim").
    pub command: String,
    /// Number of attached clients (browser tabs).
    pub clients: u32,
    /// Last output, in Unix seconds.
    pub activity: u64,
    /// Pid of the shell. Defaulted so a ptyd that predates it still parses.
    #[serde(default)]
    pub pid: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminals: Option<Vec<TerminalInfo>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Sent on an attached stream when the terminal's program has exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
}

impl Response {
    fn ok() -> Self {
        Self {
            ok: true,
            ..Default::default()
        }
    }
    fn err(e: impl std::fmt::Display) -> Self {
        Self {
            ok: false,
            error: Some(e.to_string()),
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------- framing

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    kind: u8,
    payload: &[u8],
) -> Result<()> {
    let len = u32::try_from(payload.len() + 1).context("frame too large")?;
    let mut buf = Vec::with_capacity(payload.len() + 5);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.push(kind);
    buf.extend_from_slice(payload);
    writer.write_all(&buf).await?;
    Ok(())
}

pub async fn write_json<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<()> {
    write_frame(writer, KIND_JSON, &serde_json::to_vec(value)?).await
}

/// `Ok(None)` on a clean end of stream.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<(u8, Vec<u8>)>> {
    let mut len = [0u8; 4];
    if reader.read(&mut len[..1]).await? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut len[1..]).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        bail!("bad frame length {len}");
    }
    let kind = reader.read_u8().await?;
    let mut payload = vec![0u8; len - 1];
    reader.read_exact(&mut payload).await?;
    Ok(Some((kind, payload)))
}

// ----------------------------------------------------------------- daemon

enum TerminalEvent {
    Data(Arc<[u8]>),
    Exit,
}

struct Screen {
    parser: vt100::Parser,
    clients: Vec<mpsc::Sender<TerminalEvent>>,
}

struct Session {
    id: String,
    owner: String,
    title: Mutex<String>,
    created: u64,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<std::fs::File>,
    screen: Mutex<Screen>,
    activity: AtomicU64,
    pid: Option<u32>,
    /// Cleared after the child is reaped; checked before delayed SIGKILL.
    alive: std::sync::atomic::AtomicBool,
}

type Sessions = Arc<Mutex<HashMap<String, Arc<Session>>>>;

/// A client that falls this many output chunks behind is disconnected; the
/// browser reconnects and gets a fresh snapshot instead of a stalled stream.
const CLIENT_QUEUE: usize = 1024;

pub async fn run(socket: &Path, scrollback: usize) -> Result<()> {
    // One daemon per socket: the lock is held for the life of the process.
    let lock_path = socket.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    use std::os::fd::AsRawFd;
    // SAFETY: valid fd owned by `lock`, which lives until the end of `run`.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        tracing::info!("another ptyd already serves {}", socket.display());
        return Ok(());
    }
    // We hold the lock, so an existing socket file is a leftover.
    let _ = std::fs::remove_file(socket);
    // Create the socket with no group/other access from the start (not after a
    // bind-then-chmod window): tighten the umask around bind.
    let prev_umask = unsafe { libc::umask(0o077) };
    let listener = UnixListener::bind(socket);
    unsafe { libc::umask(prev_umask) };
    let listener = listener.with_context(|| format!("binding {}", socket.display()))?;
    std::fs::set_permissions(socket, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    tracing::info!(
        "ptyd {} (protocol {PROTOCOL}) listening on {}",
        env!("CARGO_PKG_VERSION"),
        socket.display()
    );

    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    loop {
        let (stream, _) = listener.accept().await?;
        let sessions = sessions.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, sessions, scrollback).await {
                tracing::debug!("ptyd connection: {e:#}");
            }
        });
    }
}

/// The uid of the process on the other end of a Unix socket, if obtainable.
fn peer_uid(stream: &UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "linux")]
    {
        let mut credentials = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: credentials/len are valid for SO_PEERCRED on a connected socket.
        let status = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        (status == 0).then_some(credentials.uid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (mut uid, mut gid) = (0u32, 0u32);
        // SAFETY: getpeereid writes the peer's uid/gid for a connected socket.
        let status = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        (status == 0).then_some(uid)
    }
}

async fn handle(mut stream: UnixStream, sessions: Sessions, scrollback: usize) -> Result<()> {
    // Only our own uid may drive the daemon. The socket is already 0600, but
    // this also rejects a root process and makes the trust boundary explicit:
    // whoever connects controls every terminal.
    let daemon_uid = unsafe { libc::geteuid() };
    match peer_uid(&stream) {
        Some(uid) if uid == daemon_uid => {}
        other => {
            tracing::warn!("ptyd: rejected connection from uid {other:?} (expected {daemon_uid})");
            return Ok(());
        }
    }
    let Some((kind, payload)) = read_frame(&mut stream).await? else {
        return Ok(());
    };
    if kind != KIND_JSON {
        bail!("expected a request");
    }
    let req: Request = serde_json::from_slice(&payload)?;
    let resp = match req {
        Request::Hello => Response {
            protocol: Some(PROTOCOL),
            version: Some(env!("CARGO_PKG_VERSION").into()),
            ..Response::ok()
        },
        Request::List => {
            let list: Vec<Arc<Session>> = sessions.lock().unwrap().values().cloned().collect();
            Response {
                terminals: Some(list.iter().map(|session| terminal_info(session)).collect()),
                ..Response::ok()
            }
        }
        Request::Create(request) => match create(&sessions, request, scrollback) {
            Ok(id) => Response {
                id: Some(id),
                ..Response::ok()
            },
            Err(e) => Response::err(format!("{e:#}")),
        },
        Request::Kill { id } => match sessions.lock().unwrap().get(&id).cloned() {
            Some(session) => {
                hang_up(&session);
                Response::ok()
            }
            None => Response::err("no such terminal"),
        },
        Request::Rename { id, title } => match sessions.lock().unwrap().get(&id) {
            Some(session) => {
                *session.title.lock().unwrap() = title;
                Response::ok()
            }
            None => Response::err("no such terminal"),
        },
        Request::Attach { id, cols, rows } => {
            let session = sessions.lock().unwrap().get(&id).cloned();
            return match session {
                Some(session) => attach(stream, session, cols, rows).await,
                None => write_json(&mut stream, &Response::err("no such terminal")).await,
            };
        }
        Request::Resize { .. } => Response::err("resize outside of attach"),
    };
    write_json(&mut stream, &resp).await
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> String {
    let bytes: [u8; 6] = rand::random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    // Caps bound per-terminal scrollback memory (≈ rows × cols × 32 B × history).
    PtySize {
        rows: rows.clamp(2, 400),
        cols: cols.clamp(2, 500),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Environment variables a login shell legitimately needs, inherited from
/// ptyd's own environment. Everything else (including any `Environment=`
/// secrets of the systemd service, and codeenv's own vars) is dropped.
const ENV_ALLOW: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "PATH",
    "LANG",
    "LANGUAGE",
    "TZ",
    "XDG_RUNTIME_DIR",
    "SSH_AUTH_SOCK",
];

fn create(sessions: &Sessions, request: CreateRequest, scrollback: usize) -> Result<String> {
    let dimensions = pty_size(request.cols, request.rows);
    let pair = native_pty_system().openpty(dimensions).context("openpty")?;
    let mut cmd = CommandBuilder::new(&request.shell);
    cmd.arg("-l");
    cmd.cwd(&request.cwd);
    // Start from a clean environment, keep only what a shell needs, then add
    // the per-terminal vars the client asked for.
    cmd.env_clear();
    for (key, value) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|key| ENV_ALLOW.contains(&key) || key.starts_with("LC_"))
        {
            cmd.env(key, value);
        }
    }
    for (key, value) in &request.env {
        cmd.env(key, value);
    }
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("starting {}", request.shell))?;
    drop(pair.slave);
    let master = pair.master;
    let mut reader = master.try_clone_reader()?;
    let mut writer = dup_writer(master.as_ref())?;
    if let Some(command) = request
        .command
        .as_deref()
        .filter(|command| !command.trim().is_empty())
    {
        // Typed ahead: the shell reads it once its prompt is ready, so it
        // lands in the history and Ctrl-C returns to a prompt.
        writer.write_all(format!("{command}\r").as_bytes())?;
    }

    let id = new_id();
    let session = Arc::new(Session {
        id: id.clone(),
        owner: request.owner,
        title: Mutex::new(request.title),
        created: now(),
        pid: child.process_id(),
        master: Mutex::new(master),
        writer: Mutex::new(writer),
        screen: Mutex::new(Screen {
            parser: vt100::Parser::new(dimensions.rows, dimensions.cols, scrollback),
            clients: Vec::new(),
        }),
        activity: AtomicU64::new(now()),
        alive: std::sync::atomic::AtomicBool::new(true),
    });
    sessions.lock().unwrap().insert(id.clone(), session.clone());

    let sessions = sessions.clone();
    std::thread::Builder::new()
        .name(format!("pty-{id}"))
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(bytes_read) => {
                        let data: Arc<[u8]> = Arc::from(&buf[..bytes_read]);
                        session.activity.store(now(), Ordering::Relaxed);
                        // Parse and fan out under one lock, so an attaching client
                        // gets a snapshot and then exactly the bytes after it.
                        let mut screen = session.screen.lock().unwrap();
                        screen.parser.process(&data);
                        screen.clients.retain(|client| {
                            client.try_send(TerminalEvent::Data(data.clone())).is_ok()
                        });
                    }
                }
            }
            // All slave fds are closed: the program and everything it started
            // on this terminal are gone.
            let _ = child.wait();
            // Reaped: the pid may now be reused, so forbid any later SIGKILL.
            session.alive.store(false, Ordering::SeqCst);
            sessions.lock().unwrap().remove(&session.id);
            for client in session.screen.lock().unwrap().clients.drain(..) {
                let _ = client.try_send(TerminalEvent::Exit);
            }
            tracing::info!("terminal {} exited", session.id);
        })?;
    Ok(id)
}

/// Like closing a terminal window: SIGHUP to the shell and to the foreground
/// job; the shell forwards it to its other jobs.
fn hang_up(session: &Arc<Session>) {
    let foreground_group = session.master.lock().unwrap().process_group_leader();
    // SAFETY: plain signal sends to pids/process groups of this terminal.
    unsafe {
        if let Some(pid) = session.pid {
            libc::kill(pid as libc::pid_t, libc::SIGHUP);
        }
        if let Some(group_id) = foreground_group {
            libc::kill(-group_id, libc::SIGHUP);
        }
    }
    // Escalate to SIGKILL after three seconds if the child has not been reaped.
    let session = session.clone();
    let id = session.id.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("kill-{id}"))
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            if !session.alive.load(Ordering::SeqCst) {
                return;
            }
            let foreground_group = session.master.lock().unwrap().process_group_leader();
            unsafe {
                if let Some(group_id) = foreground_group {
                    libc::kill(-group_id, libc::SIGKILL);
                }
                if let Some(pid) = session.pid {
                    libc::kill(pid as libc::pid_t, libc::SIGKILL);
                }
            }
        });
    // SIGHUP has already been sent even if the follow-up thread fails.
    if let Err(e) = spawned {
        tracing::warn!("ptyd: could not schedule SIGKILL for {id}: {e}");
    }
}

fn terminal_info(session: &Session) -> TerminalInfo {
    let foreground_group = session.master.lock().unwrap().process_group_leader();
    let foreground_pid = foreground_group.map(|p| p as u32).or(session.pid);
    let clients = session.screen.lock().unwrap().clients.len() as u32;
    TerminalInfo {
        id: session.id.clone(),
        title: session.title.lock().unwrap().clone(),
        owner: session.owner.clone(),
        created: session.created,
        cwd: foreground_pid
            .and_then(crate::procinfo::cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default(),
        command: foreground_pid
            .and_then(crate::procinfo::name)
            .unwrap_or_default(),
        clients,
        activity: session.activity.load(Ordering::Relaxed),
        pid: session.pid.unwrap_or(0),
    }
}

async fn attach(stream: UnixStream, session: Arc<Session>, cols: u16, rows: u16) -> Result<()> {
    resize(&session, cols, rows);
    let (tx, mut rx) = mpsc::channel(CLIENT_QUEUE);
    let screen_bytes = {
        let mut screen = session.screen.lock().unwrap();
        let screen_bytes = snapshot(&mut screen.parser);
        screen.clients.push(tx);
        screen_bytes
    };
    let (mut reader, mut writer) = stream.into_split();
    write_json(&mut writer, &Response::ok()).await?;
    // The snapshot of a large, heavily-coloured scrollback can be tens of MB;
    // send it as bounded frames so it never exceeds the client's frame limit.
    for chunk in screen_bytes.chunks(256 * 1024) {
        write_frame(&mut writer, KIND_DATA, chunk).await?;
    }

    let output = async {
        while let Some(event) = rx.recv().await {
            match event {
                TerminalEvent::Data(data) => write_frame(&mut writer, KIND_DATA, &data).await?,
                TerminalEvent::Exit => {
                    write_json(
                        &mut writer,
                        &Response {
                            event: Some("exit".into()),
                            ..Response::ok()
                        },
                    )
                    .await?;
                    break;
                }
            }
        }
        anyhow::Ok(())
    };
    let input = async {
        while let Some((kind, payload)) = read_frame(&mut reader).await? {
            match kind {
                // Keystrokes are tiny and the PTY write returns at once; doing
                // it inline avoids spawning a task per frame.
                KIND_DATA => {
                    session.writer.lock().unwrap().write_all(&payload)?;
                }
                KIND_JSON => match serde_json::from_slice::<Request>(&payload)? {
                    Request::Resize { cols, rows } => resize(&session, cols, rows),
                    other => bail!("unexpected {other:?} on attached stream"),
                },
                kind => bail!("unknown frame kind {kind}"),
            }
        }
        anyhow::Ok(())
    };
    // Either side ending ends the attachment.
    let result = tokio::select! {
        result = output => result,
        result = input => result,
    };
    // Unsubscribe now rather than on the next output, so the client count
    // shown in the UI is right even for an idle terminal.
    drop(rx);
    session
        .screen
        .lock()
        .unwrap()
        .clients
        .retain(|c| !c.is_closed());
    result
}

/// The terminal takes the size of the most recent client to attach or resize.
fn resize(session: &Session, cols: u16, rows: u16) {
    let dimensions = pty_size(cols, rows);
    if let Err(e) = session.master.lock().unwrap().resize(dimensions) {
        tracing::warn!("terminal {}: resize: {e}", session.id);
    }
    session
        .screen
        .lock()
        .unwrap()
        .parser
        .screen_mut()
        .set_size(dimensions.rows, dimensions.cols);
}

/// Bytes that bring a freshly reset terminal to the state of `parser`:
/// scrollback (normal screen only), screen contents, cursor, input modes.
pub fn snapshot(parser: &mut vt100::Parser) -> Vec<u8> {
    let screen = parser.screen_mut();
    let (rows, cols) = screen.size();
    let mut bytes = Vec::new();
    if screen.alternate_screen() {
        // Full-screen program: its screen has no scrollback.
        bytes.extend_from_slice(b"\x1b[?1049h");
    } else {
        screen.set_scrollback(usize::MAX);
        let history_rows = screen.scrollback();
        let mut offset = history_rows;
        while offset > 0 {
            screen.set_scrollback(offset);
            let page_rows = offset.min(rows as usize);
            for row in screen.rows_formatted(0, cols).take(page_rows) {
                bytes.extend_from_slice(&row);
                bytes.extend_from_slice(b"\x1b[0m\r\n");
            }
            offset -= page_rows;
        }
        screen.set_scrollback(0);
        if history_rows > 0 {
            // Scroll the last page of history off-screen before the screen
            // is redrawn over it.
            bytes.extend(std::iter::repeat_n(b'\n', rows as usize));
        }
    }
    bytes.extend_from_slice(&screen.state_formatted());
    bytes
}

/// A writer on the PTY master. Not `take_writer()`: its Drop writes "\n" +
/// VEOF into the terminal, i.e. sends EOF to the program.
fn dup_writer(master: &dyn MasterPty) -> Result<std::fs::File> {
    use std::os::fd::FromRawFd;
    let fd = master
        .as_raw_fd()
        .ok_or_else(|| anyhow!("PTY master has no file descriptor"))?;
    // SAFETY: `fd` is open and owned by `master`; the duplicate is owned by
    // the returned File.
    let dup = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
    if dup < 0 {
        return Err(std::io::Error::last_os_error()).context("dup PTY master");
    }
    Ok(unsafe { std::fs::File::from_raw_fd(dup) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_handle_fragmentation_and_truncated_input() {
        let (mut writer, mut reader) = tokio::io::duplex(1);
        let sent = tokio::spawn(async move {
            write_frame(&mut writer, KIND_DATA, b"hello").await.unwrap();
            write_frame(&mut writer, KIND_JSON, b"{}").await.unwrap();
        });
        assert_eq!(
            read_frame(&mut reader).await.unwrap(),
            Some((KIND_DATA, b"hello".to_vec()))
        );
        assert_eq!(
            read_frame(&mut reader).await.unwrap(),
            Some((KIND_JSON, b"{}".to_vec()))
        );
        assert!(read_frame(&mut reader).await.unwrap().is_none());
        sent.await.unwrap();

        for bytes in [&b"\0\0"[..], &b"\0\0\0\x04\x02hi"[..], &b"\0\0\0\0"[..]] {
            assert!(read_frame(&mut &*bytes).await.is_err());
        }
    }

    /// Replaying a snapshot into a fresh terminal reproduces scrollback,
    /// screen and input modes.
    #[test]
    fn snapshot_roundtrip() {
        let mut original = vt100::Parser::new(5, 20, 100);
        for i in 0..12 {
            original.process(format!("\x1b[31mline {i}\x1b[0m\r\n").as_bytes());
        }
        original.process(b"$ \x1b[?1000h\x1b[?1006h\x1b[?2004h");
        let screen_bytes = snapshot(&mut original);
        assert_eq!(original.screen().scrollback(), 0);

        let mut restored = vt100::Parser::new(5, 20, 100);
        restored.process(&screen_bytes);
        assert_eq!(restored.screen().contents(), original.screen().contents());
        assert_eq!(
            restored.screen().cursor_position(),
            original.screen().cursor_position()
        );
        assert_eq!(
            restored.screen().mouse_protocol_mode(),
            vt100::MouseProtocolMode::PressRelease
        );
        assert_eq!(
            restored.screen().mouse_protocol_encoding(),
            vt100::MouseProtocolEncoding::Sgr
        );
        assert!(restored.screen().bracketed_paste());
        assert_eq!(
            restored.screen().cell(0, 0).unwrap().fgcolor(),
            original.screen().cell(0, 0).unwrap().fgcolor()
        );
        // Whole history present above the screen.
        restored.screen_mut().set_scrollback(usize::MAX);
        let history = restored.screen().contents();
        assert!(history.starts_with("line 0"), "{history:?}");

        // Alternate screen: switched to, drawn, no history replay.
        let mut original = vt100::Parser::new(5, 20, 100);
        original.process(b"history\r\n\x1b[?1049h\x1b[2;3Hvim");
        let mut restored = vt100::Parser::new(5, 20, 100);
        restored.process(&snapshot(&mut original));
        assert!(restored.screen().alternate_screen());
        assert_eq!(restored.screen().contents(), original.screen().contents());
    }
}
