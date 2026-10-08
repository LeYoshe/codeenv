//! Exercises the production router over TCP. Only the Access key cache is seeded
//! locally; signature verification and middleware are the production code.
use super::*;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{any, get};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, UnixListener};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

struct Server {
    url: String,
    task: JoinHandle<()>,
}
impl Server {
    async fn start(router: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { url, task }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ce-test-{:016x}", rand::random::<u64>()));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn app(workspace: &Workspace) -> Arc<App> {
    let config: Config = toml::from_str(
        r#"
hosts = ["workspace.test"]
[auth]
mode = "cloudflare_access"
team_domain = "tests"
audiences = ["codeenv-tests"]
[ports]
host_template = "p{port}.workspace.test"
deny = [22]
"#,
    )
    .unwrap();
    Arc::new(App {
        auth: auth::test_auth().await,
        pty: pty::PtyClient::for_test(workspace.0.join("ptyd.sock")),
        store: store::Store::new(&workspace.0).unwrap(),
        root: workspace.0.clone(),
        home: workspace.0.clone(),
        name: "Tests".into(),
        host_template: Some(HostTemplate::parse("p{port}.workspace.test").unwrap()),
        proxy: proxy::client(),
        own_port: 7681,
        config,
    })
}
fn token(email: &str) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test".into());
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    jsonwebtoken::encode(&header, &json!({"iss":"https://tests.cloudflareaccess.com", "aud":"codeenv-tests", "email":email, "exp":exp}),
        &jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("testdata/access_test_key.pem")).unwrap()).unwrap()
}
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}
fn request(server: &Server, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
    client()
        .request(method, format!("{}{path}", server.url))
        .header("host", "workspace.test")
        .header(auth::JWT_HEADER, token("alice@example.com"))
}
fn websocket(server: &Server, path: &str, host: &str, origin: &str) -> axum::http::Request<()> {
    let mut req = format!("{}{path}", server.url.replace("http:", "ws:"))
        .into_client_request()
        .unwrap();
    req.headers_mut().insert("host", host.parse().unwrap());
    req.headers_mut().insert("origin", origin.parse().unwrap());
    req.headers_mut().insert(
        auth::JWT_HEADER,
        token("alice@example.com").parse().unwrap(),
    );
    req
}
async fn ws_status(req: axum::http::Request<()>) -> StatusCode {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(req),
    )
    .await
    .unwrap();
    match result {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => response.status(),
        other => panic!("expected rejected upgrade: {other:?}"),
    }
}

#[tokio::test]
async fn http_authentication_and_origin_enforcement() {
    let workspace = Workspace::new();
    let server = Server::start(router(app(&workspace).await)).await;
    for path in ["/", "/api/me", "/api/fs", "/api/fs/download?path=missing"] {
        for jwt in ["", "invalid.jwt.token"] {
            let response = client()
                .get(format!("{}{path}", server.url))
                .header("host", "workspace.test")
                .header(auth::JWT_HEADER, jwt)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        }
    }
    let response = request(&server, reqwest::Method::GET, "/api/me")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.text().await.unwrap().contains("alice@example.com"));
    assert_eq!(
        request(&server, reqwest::Method::GET, "/api/me")
            .headers(host_header("attacker.test"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::MISDIRECTED_REQUEST
    );
    for origin in [
        None,
        Some("null"),
        Some("https://attacker.test"),
        Some("https://p3000.workspace.test"),
    ] {
        let mut req = request(&server, reqwest::Method::PUT, "/api/buttons").json(&json!([]));
        if let Some(origin) = origin {
            req = req.header("origin", origin);
        }
        assert_eq!(req.send().await.unwrap().status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(
        request(&server, reqwest::Method::PUT, "/api/buttons")
            .header("origin", "https://workspace.test")
            .json(&json!([]))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        request(&server, reqwest::Method::GET, "/api/fs")
            .header("sec-fetch-site", "cross-site")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn websocket_authentication_and_origin_enforcement() {
    let workspace = Workspace::new();
    let server = Server::start(router(app(&workspace).await)).await;
    let mut req = websocket(
        &server,
        "/api/terminals/other/ws",
        "workspace.test",
        "https://workspace.test",
    );
    req.headers_mut().remove(auth::JWT_HEADER);
    assert_eq!(ws_status(req).await, StatusCode::FORBIDDEN);
    for origin in [
        "null",
        "https://attacker.test",
        "https://p3000.workspace.test",
    ] {
        assert_eq!(
            ws_status(websocket(
                &server,
                "/api/terminals/other/ws",
                "workspace.test",
                origin
            ))
            .await,
            StatusCode::FORBIDDEN
        );
    }
    let mut req = websocket(
        &server,
        "/api/terminals/other/ws",
        "workspace.test",
        "https://workspace.test",
    );
    req.headers_mut().remove("origin");
    assert_eq!(ws_status(req).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn another_users_terminal_is_hidden_and_cannot_be_modified_or_attached() {
    let workspace = Workspace::new();
    let listener = UnixListener::bind(workspace.0.join("ptyd.sock")).unwrap();
    // The daemon fixture accepts only list requests. Any mutation fails the test.
    let daemon = tokio::spawn(async move {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (_, bytes) = ptyd::read_frame(&mut stream).await.unwrap().unwrap();
            assert!(matches!(
                serde_json::from_slice::<ptyd::Request>(&bytes).unwrap(),
                ptyd::Request::List
            ));
            ptyd::write_json(&mut stream,&json!({"ok":true,"terminals":[{"id":"other","owner":"bob@example.com","title":"Private","created":0,"cwd":"/","command":"bash","clients":0,"activity":0}]})).await.unwrap();
        }
    });
    let server = Server::start(router(app(&workspace).await)).await;
    let response = request(&server, reqwest::Method::GET, "/api/terminals")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.json::<Value>().await.unwrap(), json!([]));
    for method in [reqwest::Method::PATCH, reqwest::Method::DELETE] {
        let response = request(&server, method, "/api/terminals/other")
            .header("origin", "https://workspace.test")
            .json(&json!({"title":"Changed"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    assert_eq!(
        ws_status(websocket(
            &server,
            "/api/terminals/other/ws",
            "workspace.test",
            "https://workspace.test"
        ))
        .await,
        StatusCode::NOT_FOUND
    );
    tokio::time::timeout(Duration::from_secs(5), daemon)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn proxy_forwards_http_without_access_credentials_or_parent_domain_cookies() {
    let upstream = Server::start(Router::new().fallback(any(|headers: HeaderMap, uri: axum::http::Uri, body: String| async move {
        let response = json!({"headers":headers.iter().map(|(k,v)|(k.as_str().to_string(),v.to_str().unwrap().to_string())).collect::<std::collections::BTreeMap<_,_>>(),"uri":uri.to_string(),"body":body});
        ([("set-cookie","session=abc; Domain=workspace.test; Path=/; HttpOnly"),("connection","x-private"),("x-private","remove-me")],axum::Json(response))
    }))).await;
    let port = upstream.url.rsplit(':').next().unwrap().to_string();
    let workspace = Workspace::new();
    let server = Server::start(router(app(&workspace).await)).await;
    let response = request(&server, reqwest::Method::POST, "/echo?q=hello%20world")
        .headers(host_header(&format!("p{port}.workspace.test")))
        .header("cookie", "CF_Authorization=secret; app=keep")
        .header("x-forwarded-host", "attacker.test")
        .header("connection", "x-private")
        .header("x-private", "remove-me")
        .body("payload")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["set-cookie"],
        "session=abc; Path=/; HttpOnly"
    );
    assert!(!response.headers().contains_key("x-private"));
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["uri"], "/echo?q=hello%20world");
    assert_eq!(body["body"], "payload");
    assert_eq!(body["headers"]["host"], format!("localhost:{port}"));
    assert_eq!(
        body["headers"]["x-forwarded-host"],
        format!("p{port}.workspace.test")
    );
    assert_eq!(body["headers"]["cookie"], "app=keep");
    assert!(body["headers"].get(auth::JWT_HEADER).is_none());
    assert!(body["headers"].get("x-private").is_none());
    for forbidden in [22, 7681] {
        assert_eq!(
            request(&server, reqwest::Method::GET, "/")
                .headers(host_header(&format!("p{forbidden}.workspace.test")))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    drop(upstream);
    let response = request(&server, reqwest::Method::GET, "/")
        .headers(host_header(&format!("p{port}.workspace.test")))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["cache-control"], "no-store");
}

#[tokio::test]
async fn proxy_websocket_exchanges_text_binary_and_close_frames() {
    let upstream = Server::start(Router::new().route(
        "/ws",
        get(|ws: axum::extract::WebSocketUpgrade| async {
            ws.on_upgrade(|mut socket| async move {
                while let Some(Ok(message)) = socket.recv().await {
                    let closing = matches!(message, axum::extract::ws::Message::Close(_));
                    if closing {
                        let _ = socket.close().await;
                        break;
                    }
                    if socket.send(message).await.is_err() {
                        break;
                    }
                }
            })
        }),
    ))
    .await;
    let port = upstream.url.rsplit(':').next().unwrap().to_string();
    let workspace = Workspace::new();
    let server = Server::start(router(app(&workspace).await)).await;
    let req = websocket(
        &server,
        "/ws",
        &format!("p{port}.workspace.test"),
        "https://workspace.test",
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    for message in [
        Message::Text("hello".into()),
        Message::Binary(vec![0, 1, 255].into()),
    ] {
        socket.send(message.clone()).await.unwrap();
        let echoed = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(echoed, message);
    }
    socket.close(None).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap();
    assert!(matches!(reply, Some(Ok(Message::Close(_)))));
}

fn upload_params(path: &str, id: &str, offset: u64, done: bool) -> transfer::UploadParams {
    transfer::UploadParams {
        dir: String::new(),
        path: path.into(),
        id: id.into(),
        offset,
        done,
        overwrite: false,
    }
}

#[tokio::test]
async fn interrupted_upload_can_retry_without_losing_previous_chunks() {
    use axum::body::{Body, Bytes};
    let workspace = Workspace::new();
    transfer::upload_chunk(
        &workspace.0,
        upload_params("file.txt", "retry", 0, false),
        Body::from("first"),
    )
    .await
    .unwrap();
    let stream = futures_util::stream::iter(vec![
        Ok(Bytes::from_static(b"partial")),
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "client disconnected",
        )),
    ]);
    assert!(
        transfer::upload_chunk(
            &workspace.0,
            upload_params("file.txt", "retry", 5, true),
            Body::from_stream(stream)
        )
        .await
        .is_err()
    );
    assert!(!workspace.0.join("file.txt").exists());
    transfer::upload_chunk(
        &workspace.0,
        upload_params("file.txt", "retry", 5, true),
        Body::from(" second"),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(workspace.0.join("file.txt")).unwrap(),
        b"first second"
    );
}

#[tokio::test]
async fn cancelling_an_upload_preserves_destination_and_other_uploads() {
    use axum::body::Body;
    let workspace = Workspace::new();
    std::fs::write(workspace.0.join("file.txt"), "original").unwrap();
    for id in ["cancel", "keep"] {
        transfer::upload_chunk(
            &workspace.0,
            upload_params("file.txt", id, 0, false),
            Body::from(id),
        )
        .await
        .unwrap();
    }
    transfer::upload_abort(&workspace.0, &upload_params("file.txt", "cancel", 0, false))
        .await
        .unwrap();
    transfer::upload_abort(&workspace.0, &upload_params("file.txt", "cancel", 0, false))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(workspace.0.join("file.txt")).unwrap(),
        b"original"
    );
    assert!(!workspace.0.join(".file.txt.ce-upload-cancel").exists());
    let mut params = upload_params("file.txt", "keep", 4, true);
    params.overwrite = true;
    transfer::upload_chunk(&workspace.0, params, Body::from(" completed"))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(workspace.0.join("file.txt")).unwrap(),
        b"keep completed"
    );
}

#[tokio::test]
async fn parallel_uploads_create_shared_directories_without_mixing_contents() {
    use axum::body::Body;
    let workspace = Workspace::new();
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..12 {
        let root = workspace.0.clone();
        tasks.spawn(async move {
            let path = format!("shared/nested/{index}.txt");
            transfer::upload_chunk(
                &root,
                upload_params(&path, &format!("upload-{index}"), 0, true),
                Body::from(format!("content-{index}")),
            )
            .await
            .unwrap();
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    for index in 0..12 {
        assert_eq!(
            std::fs::read_to_string(workspace.0.join(format!("shared/nested/{index}.txt")))
                .unwrap(),
            format!("content-{index}")
        );
    }
}

#[tokio::test]
async fn concurrent_uploads_cannot_overwrite_a_destination_without_permission() {
    use axum::body::Body;
    let workspace = Workspace::new();
    let (first, second) = tokio::join!(
        transfer::upload_chunk(
            &workspace.0,
            upload_params("file.txt", "one", 0, true),
            Body::from("one")
        ),
        transfer::upload_chunk(
            &workspace.0,
            upload_params("file.txt", "two", 0, true),
            Body::from("two")
        ),
    );
    assert_ne!(
        first.is_ok(),
        second.is_ok(),
        "exactly one upload must succeed"
    );
    let expected = if first.is_ok() { "one" } else { "two" };
    assert_eq!(
        std::fs::read_to_string(workspace.0.join("file.txt")).unwrap(),
        expected
    );
}

#[tokio::test]
async fn failed_upload_finalization_preserves_existing_directory() {
    use axum::body::Body;
    let workspace = Workspace::new();
    std::fs::create_dir(workspace.0.join("destination")).unwrap();
    std::fs::write(workspace.0.join("destination/keep.txt"), "keep").unwrap();
    assert!(
        transfer::upload_chunk(
            &workspace.0,
            upload_params("destination", "fail", 0, true),
            Body::from("replacement")
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(workspace.0.join("destination/keep.txt")).unwrap(),
        "keep"
    );
    assert!(!workspace.0.join(".destination.ce-upload-fail").exists());
}

#[tokio::test]
async fn preferences_survive_reopening_and_reject_invalid_updates() {
    let workspace = Workspace::new();
    let store = store::Store::new(&workspace.0).unwrap();
    let button: config::Button =
        serde_json::from_value(json!({"name":" Test ","command":"cargo test"})).unwrap();
    let saved = store
        .set_buttons("alice@example.com", vec![button.clone()])
        .await
        .unwrap();
    assert_eq!(saved[0].name, "Test");
    assert!(!saved[0].id.is_empty());
    let mut invalid = button;
    invalid.command = " ".into();
    assert!(
        store
            .set_buttons("alice@example.com", vec![invalid])
            .await
            .is_err()
    );
    let reopened = store::Store::new(&workspace.0).unwrap();
    assert_eq!(
        reopened.load("alice@example.com").await.unwrap().buttons[0].id,
        saved[0].id
    );
    assert!(
        reopened
            .load("bob@example.com")
            .await
            .unwrap()
            .buttons
            .is_empty()
    );
    std::fs::write(
        workspace.0.join("users/alice@example.com.json"),
        "invalid JSON",
    )
    .unwrap();
    assert!(reopened.load("alice@example.com").await.is_err());
    assert!(
        reopened
            .set_buttons("alice@example.com", vec![])
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(workspace.0.join("users/alice@example.com.json")).unwrap(),
        "invalid JSON"
    );
}

fn host_header(host: &str) -> HeaderMap {
    HeaderMap::from_iter([(axum::http::header::HOST, host.parse().unwrap())])
}

#[tokio::test]
async fn concurrent_editor_saves_reject_stale_versions() {
    let workspace = Workspace::new();
    std::fs::write(workspace.0.join("note.txt"), "original").unwrap();
    let version = files::read_text(&workspace.0, "note.txt").unwrap().version;
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..8 {
        let root = workspace.0.clone();
        let version = version.clone();
        let barrier = barrier.clone();
        tasks.spawn_blocking(move || {
            barrier.wait();
            let content = format!("editor-{index}");
            files::write_text(&root, "note.txt", &content, Some(&version)).map(|_| content)
        });
    }
    let mut winners = Vec::new();
    while let Some(result) = tasks.join_next().await {
        if let Ok(content) = result.unwrap() {
            winners.push(content);
        }
    }
    assert_eq!(
        winners.len(),
        1,
        "concurrent edits must not silently overwrite each other"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.0.join("note.txt")).unwrap(),
        winners[0]
    );
}
