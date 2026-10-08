//! JSON API used by the web UI.

use std::sync::Arc;

use axum::extract::{Path, Query, Request, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderValue};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Extension, Json, Router, middleware};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::App;
use crate::auth::{User, same_origin};
use crate::config::Button;
use crate::{files, ports, ptyd, search, transfer};

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/me", get(me))
        .route("/api/fs", get(fs_list))
        .route(
            "/api/fs/file",
            get(fs_read)
                .put(fs_write)
                .layer(axum::extract::DefaultBodyLimit::max(
                    (files::MAX_EDIT_SIZE as usize) * 2 + 4096,
                )),
        )
        .route("/api/fs/create", axum::routing::post(fs_create))
        .route("/api/fs/mkdirs", axum::routing::post(fs_mkdirs))
        .route("/api/fs/rename", axum::routing::post(fs_rename))
        .route("/api/fs/delete", axum::routing::post(fs_delete))
        .route(
            "/api/fs/upload",
            axum::routing::put(fs_upload).delete(fs_upload_abort),
        )
        .route("/api/fs/download", get(fs_download))
        .route("/api/fs/raw", get(fs_raw))
        .route("/api/search/text", get(search_text))
        .route("/api/search/files", get(search_files))
        .route("/api/terminals", get(terminals).post(create_terminal))
        .route(
            "/api/terminals/{id}",
            patch(rename_terminal).delete(kill_terminal),
        )
        .route("/api/terminals/{id}/ws", get(terminal_ws))
        .route("/api/buttons", axum::routing::put(set_buttons))
        .route("/api/ports", get(list_ports))
        // Harden every API response, then reject cross-site callers. Order:
        // headers is added last, so it is the outer layer and also covers the
        // rejections produced by same_origin.
        .layer(middleware::from_fn(same_origin))
        .layer(middleware::from_fn(api_headers))
}

/// Prevents caching, MIME sniffing and cross-origin use of API responses.
async fn api_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    response
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        if let Some(http_error) = e.downcast_ref::<files::HttpError>() {
            return ApiError(http_error.0, http_error.1.clone());
        }
        tracing::warn!("api error: {e:#}");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn not_found() -> ApiError {
    ApiError(StatusCode::NOT_FOUND, "terminal not found".into())
}

type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Serialize)]
struct UserInfo {
    email: String,
    name: String,
    root: String,
    home: String,
    shell: String,
    global_buttons: Vec<Button>,
    buttons: Vec<Button>,
    /// Subdomain template for port forwarding, or null when it is disabled.
    port_host_template: Option<String>,
}

async fn me(State(app): State<Arc<App>>, Extension(user): Extension<User>) -> ApiResult<UserInfo> {
    let data = app.store.load(&user.email).await?;
    Ok(Json(UserInfo {
        email: user.email,
        name: app.name.clone(),
        root: app.root.to_string_lossy().into_owned(),
        home: app.home.to_string_lossy().into_owned(),
        shell: app.pty.shell().to_string(),
        global_buttons: app.config.buttons.clone(),
        buttons: data.buttons,
        port_host_template: app.host_template.as_ref().map(|t| t.template()),
    }))
}

#[derive(Deserialize)]
struct PathQuery {
    #[serde(default)]
    path: String,
}

async fn fs_list(
    State(app): State<Arc<App>>,
    Query(query): Query<PathQuery>,
) -> ApiResult<files::Listing> {
    let root = app.root.clone();
    let listing = tokio::task::spawn_blocking(move || files::list(&root, &query.path))
        .await
        .map_err(|e| anyhow::anyhow!(e))?
        .map_err(|e| bad_request(format!("{e:#}")))?;
    Ok(Json(listing))
}

async fn terminals(
    State(app): State<Arc<App>>,
    Extension(user): Extension<User>,
) -> ApiResult<Vec<ptyd::TerminalInfo>> {
    Ok(Json(app.pty.list_for(&user.email).await?))
}

#[derive(Deserialize)]
struct CreateTerminal {
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    command: String,
}

async fn create_terminal(
    State(app): State<Arc<App>>,
    Extension(user): Extension<User>,
    Json(request): Json<CreateTerminal>,
) -> ApiResult<ptyd::TerminalInfo> {
    if app.pty.list_for(&user.email).await?.len() >= app.config.max_terminals {
        return Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "limit of {} terminals reached; close some before opening more",
                app.config.max_terminals
            ),
        ));
    }
    let cwd = files::terminal_cwd(&app.root, &app.home, &request.cwd)
        .map_err(|e| bad_request(format!("{e:#}")))?;
    let title = if request.title.trim().is_empty() {
        cwd.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into())
    } else {
        request.title
    };
    let command = Some(request.command.as_str()).filter(|c| !c.trim().is_empty());
    let id = app.pty.create(&user.email, &title, &cwd, command).await?;
    let info = app.pty.get(&user.email, &id).await?.ok_or_else(not_found)?;
    Ok(Json(info))
}

#[derive(Deserialize)]
struct RenameTerminal {
    title: String,
}

async fn rename_terminal(
    State(app): State<Arc<App>>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
    Json(request): Json<RenameTerminal>,
) -> ApiResult<serde_json::Value> {
    app.pty.get(&user.email, &id).await?.ok_or_else(not_found)?;
    app.pty.rename(&id, &request.title).await?;
    Ok(Json(json!({})))
}

async fn kill_terminal(
    State(app): State<Arc<App>>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
) -> ApiResult<serde_json::Value> {
    app.pty.get(&user.email, &id).await?.ok_or_else(not_found)?;
    app.pty.kill(&id).await?;
    Ok(Json(json!({})))
}

#[derive(Deserialize)]
struct TerminalSize {
    cols: Option<u16>,
    rows: Option<u16>,
}

async fn terminal_ws(
    State(app): State<Arc<App>>,
    Extension(user): Extension<User>,
    Path(id): Path<String>,
    Query(query): Query<TerminalSize>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    app.pty.get(&user.email, &id).await?.ok_or_else(not_found)?;
    let (cols, rows) = (query.cols.unwrap_or(120), query.rows.unwrap_or(32));
    Ok(ws.on_upgrade(move |socket| crate::terminal::serve(app, id, socket, cols, rows)))
}

async fn set_buttons(
    State(app): State<Arc<App>>,
    Extension(user): Extension<User>,
    Json(buttons): Json<Vec<Button>>,
) -> ApiResult<Vec<Button>> {
    let saved = app
        .store
        .set_buttons(&user.email, buttons)
        .await
        .map_err(|e| bad_request(format!("{e:#}")))?;
    Ok(Json(saved))
}

#[derive(Serialize)]
struct PortInfo {
    #[serde(flatten)]
    listener: ports::Listener,
    /// Public forwarding URL, or null when port forwarding is not configured.
    url: Option<String>,
    /// Full command line of the listening process.
    cmdline: String,
    /// Its working directory (usually the project).
    cwd: String,
    /// The user's terminal it was started from, if any.
    terminal: Option<PortTerminal>,
}

#[derive(Serialize)]
struct PortTerminal {
    id: String,
    title: String,
}

async fn list_ports(
    State(app): State<Arc<App>>,
    Extension(user): Extension<User>,
) -> ApiResult<Vec<PortInfo>> {
    let terminals = app.pty.list_for(&user.email).await?;
    let mut ports: Vec<PortInfo> = tokio::task::spawn_blocking(move || {
        ports::listeners()
            .into_iter()
            .map(|listener| {
                let (cmdline, cwd, terminal) = if listener.pid == 0 {
                    (String::new(), String::new(), None)
                } else {
                    let terminal = terminals
                        .iter()
                        .find(|terminal| {
                            terminal.pid != 0
                                && crate::procinfo::descends_from(listener.pid, terminal.pid)
                        })
                        .map(|terminal| PortTerminal {
                            id: terminal.id.clone(),
                            title: terminal.title.clone(),
                        });
                    (
                        crate::procinfo::cmdline(listener.pid).unwrap_or_default(),
                        crate::procinfo::cwd(listener.pid)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        terminal,
                    )
                };
                PortInfo {
                    url: None,
                    listener,
                    cmdline,
                    cwd,
                    terminal,
                }
            })
            .collect()
    })
    .await
    .map_err(|e| anyhow::anyhow!(e))?;
    ports.retain(|port_info| crate::proxy::port_allowed(&app, port_info.listener.port));
    for port_info in &mut ports {
        // A forwarding URL only exists when subdomain forwarding is configured.
        port_info.url = app
            .host_template
            .as_ref()
            .map(|template| format!("https://{}/", template.host_for(port_info.listener.port)));
    }
    Ok(Json(ports))
}

async fn fs_read(
    State(app): State<Arc<App>>,
    Query(query): Query<PathQuery>,
) -> ApiResult<files::TextFile> {
    let root = app.root.clone();
    Ok(Json(
        tokio::task::spawn_blocking(move || files::read_text(&root, &query.path))
            .await
            .map_err(anyhow::Error::from)??,
    ))
}

#[derive(Deserialize)]
struct WriteFile {
    path: String,
    content: String,
    /// Version read by the editor; omitted to overwrite unconditionally.
    version: Option<String>,
}

async fn fs_write(
    State(app): State<Arc<App>>,
    Json(request): Json<WriteFile>,
) -> ApiResult<serde_json::Value> {
    let root = app.root.clone();
    let version = tokio::task::spawn_blocking(move || {
        files::write_text(
            &root,
            &request.path,
            &request.content,
            request.version.as_deref(),
        )
    })
    .await
    .map_err(anyhow::Error::from)??;
    Ok(Json(json!({ "version": version })))
}

#[derive(Deserialize)]
struct CreateEntry {
    path: String,
    #[serde(default)]
    dir: bool,
}

async fn fs_create(
    State(app): State<Arc<App>>,
    Json(request): Json<CreateEntry>,
) -> ApiResult<serde_json::Value> {
    let path = files::create(&app.root, &request.path, request.dir)?;
    Ok(Json(json!({ "path": path })))
}

async fn fs_upload(
    State(app): State<Arc<App>>,
    Query(query): Query<transfer::UploadParams>,
    body: axum::body::Body,
) -> ApiResult<transfer::UploadResult> {
    Ok(Json(transfer::upload_chunk(&app.root, query, body).await?))
}

async fn fs_upload_abort(
    State(app): State<Arc<App>>,
    Query(query): Query<transfer::UploadParams>,
) -> ApiResult<serde_json::Value> {
    transfer::upload_abort(&app.root, &query).await?;
    Ok(Json(json!({})))
}

async fn fs_raw(
    State(app): State<Arc<App>>,
    Query(query): Query<PathQuery>,
) -> Result<Response, ApiError> {
    Ok(transfer::raw(&app.root, &query.path).await?)
}

async fn fs_download(
    State(app): State<Arc<App>>,
    Query(query): Query<PathQuery>,
) -> Result<Response, ApiError> {
    Ok(transfer::download(&app.root, &query.path).await?)
}

#[derive(Deserialize)]
struct Mkdirs {
    dir: String,
    /// "a/b/c", relative to `dir`.
    path: String,
}

async fn fs_mkdirs(
    State(app): State<Arc<App>>,
    Json(request): Json<Mkdirs>,
) -> ApiResult<serde_json::Value> {
    let dir = files::resolve(&app.root, &request.dir)?;
    let parts: Vec<&str> = request.path.split('/').filter(|s| !s.is_empty()).collect();
    let directory = transfer::ensure_dirs(&app.root, dir, &parts).await?;
    Ok(Json(json!({ "path": directory.to_string_lossy() })))
}

#[derive(Deserialize)]
struct RenameEntry {
    from: String,
    to: String,
}

async fn fs_rename(
    State(app): State<Arc<App>>,
    Json(request): Json<RenameEntry>,
) -> ApiResult<serde_json::Value> {
    let root = app.root.clone();
    let path =
        tokio::task::spawn_blocking(move || files::rename(&root, &request.from, &request.to))
            .await
            .map_err(anyhow::Error::from)??;
    Ok(Json(json!({ "path": path })))
}

async fn fs_delete(
    State(app): State<Arc<App>>,
    Json(request): Json<PathQuery>,
) -> ApiResult<serde_json::Value> {
    let root = app.root.clone();
    tokio::task::spawn_blocking(move || files::delete(&root, &request.path))
        .await
        .map_err(anyhow::Error::from)??;
    Ok(Json(json!({})))
}

/// At most this many content searches run at once, across all users: each
/// one can fan out to every CPU, so unbounded concurrency is a DoS.
static SEARCHES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// NDJSON stream of matches, file by file; ends with a `{"done":true}` line.
async fn search_text(
    State(app): State<Arc<App>>,
    Query(query): Query<search::TextQuery>,
) -> Result<Response, ApiError> {
    let dir = files::resolve(&app.root, &query.dir)?;
    let matcher = search::build_matcher(&query)?;
    let permit = SEARCHES.try_acquire().map_err(|_| {
        ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            "too many concurrent searches, try again".into(),
        )
    })?;
    let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    tokio::task::spawn_blocking(move || {
        let _permit = permit; // released when the search ends
        search::search_text(dir, matcher, tx)
    });
    // Dropping the body (client aborted) closes the channel and stops the walk.
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|chunk| (Ok::<_, std::convert::Infallible>(chunk), rx))
    });
    let mut response = axum::body::Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/x-ndjson"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

#[derive(Deserialize)]
struct NameQuery {
    #[serde(default)]
    dir: String,
    #[serde(rename = "q")]
    text: String,
}

async fn search_files(
    State(app): State<Arc<App>>,
    Query(query): Query<NameQuery>,
) -> ApiResult<Vec<search::NameHit>> {
    let dir = files::resolve(&app.root, &query.dir)?;
    let hits = tokio::task::spawn_blocking(move || search::search_names(&dir, &query.text, 50))
        .await
        .map_err(anyhow::Error::from)?;
    Ok(Json(hits))
}
