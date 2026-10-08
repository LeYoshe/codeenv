//! Reverse proxy from the tunnel to local ports, including WebSocket
//! upgrades (HMR, live reload, notebooks…).
//!
//! Subdomain addressing only: https://p5173-vps1.example.com/x →
//! http://localhost:5173/x. Each forwarded port lives on its own origin,
//! isolated from the UI's origin — there is no same-origin path mode.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};

use crate::App;
use crate::auth::{JWT_COOKIE, JWT_HEADER};

pub type HttpClient = Client<HttpConnector, Body>;

pub fn client() -> HttpClient {
    let mut connector = HttpConnector::new();
    connector.set_nodelay(true);
    // `localhost` resolves to ::1 and 127.0.0.1; try both.
    connector.set_happy_eyeballs_timeout(Some(std::time::Duration::from_millis(200)));
    connector.set_connect_timeout(Some(std::time::Duration::from_secs(5)));
    Client::builder(TokioExecutor::new()).build(connector)
}

/// Headers that apply to a single connection and must not be forwarded
/// (RFC 9110 §7.6.1). `upgrade`/`connection` are re-added for upgrades.
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub fn port_allowed(app: &App, port: u16) -> bool {
    port != 0 && port != app.own_port && !app.config.ports.deny.contains(&port)
}

pub async fn forward(app: Arc<App>, mut request: Request, port: u16) -> Response {
    if !port_allowed(&app, port) {
        return (
            StatusCode::FORBIDDEN,
            format!("forwarding port {port} is not allowed\n"),
        )
            .into_response();
    }

    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    let uri: Uri = match format!("http://localhost:{port}{path_and_query}").parse() {
        Ok(u) => u,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("bad path: {e}\n")).into_response(),
    };

    let public_host = request.headers().get(header::HOST).cloned();
    let scheme = request
        .headers()
        .get("x-forwarded-proto")
        .cloned()
        .unwrap_or(HeaderValue::from_static("http"));
    let upgrade = upgrade_protocol(request.headers());

    let mut headers = request.headers().clone();
    strip_hop_by_hop(&mut headers);
    // The Access token is for us, not for the app.
    headers.remove(JWT_HEADER);
    strip_cookie(&mut headers, JWT_COOKIE);
    // Never trust a client-supplied X-Forwarded-Host; set it ourselves.
    headers.remove("x-forwarded-host");
    if app.config.ports.rewrite_host {
        headers.insert(
            header::HOST,
            HeaderValue::from_str(&format!("localhost:{port}")).unwrap(),
        );
        if let Some(host_value) = &public_host {
            headers.insert("x-forwarded-host", host_value.clone());
        }
    }
    headers.insert("x-forwarded-proto", scheme);
    if let Some(protocol) = &upgrade {
        headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
        headers.insert(header::UPGRADE, protocol.clone());
    }

    let client_upgrade = upgrade.as_ref().map(|_| hyper::upgrade::on(&mut request));
    let (parts, body) = request.into_parts();
    let mut upstream_request = axum::http::Request::builder()
        .method(parts.method)
        .uri(uri)
        .version(axum::http::Version::HTTP_11);
    *upstream_request.headers_mut().unwrap() = headers;
    let upstream_request = upstream_request.body(body).unwrap();

    let mut response = match app.proxy.request(upstream_request).await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("proxy to port {port}: {e:?}");
            return unreachable_page(port);
        }
    };

    if response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let Some(client_upgrade) = client_upgrade else {
            return (
                StatusCode::BAD_GATEWAY,
                "unexpected upgrade from upstream\n",
            )
                .into_response();
        };
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        tokio::spawn(async move {
            match tokio::try_join!(client_upgrade, upstream_upgrade) {
                Ok((client_stream, upstream_stream)) => {
                    let (mut client_stream, mut upstream_stream) =
                        (TokioIo::new(client_stream), TokioIo::new(upstream_stream));
                    let _ = tokio::io::copy_bidirectional(&mut client_stream, &mut upstream_stream)
                        .await;
                }
                Err(e) => tracing::debug!("proxy upgrade on port {port}: {e}"),
            }
        });
        let (mut parts, _) = response.into_parts();
        strip_cookie_domain(&mut parts.headers);
        return Response::from_parts(parts, Body::empty());
    }

    let (mut parts, body) = response.into_parts();
    strip_hop_by_hop(&mut parts.headers);
    // A forwarded app must not set cookies scoped to the parent domain (which
    // would reach the UI and sibling port subdomains); keep them host-only.
    strip_cookie_domain(&mut parts.headers);
    Response::from_parts(parts, Body::new(body))
}

/// Removes the `Domain=` attribute from every Set-Cookie, so cookies set by a
/// forwarded app apply only to that app's own hostname.
fn strip_cookie_domain(headers: &mut HeaderMap) {
    let rewritten: Vec<HeaderValue> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| {
            let Ok(cookie) = value.to_str() else {
                return value.clone();
            };
            let kept: Vec<&str> = cookie
                .split(';')
                .map(str::trim)
                .enumerate()
                .filter(|(i, attribute)| {
                    *i == 0
                        || !attribute
                            .split_once('=')
                            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case("domain"))
                })
                .map(|(_, attribute)| attribute)
                .collect();
            HeaderValue::from_str(&kept.join("; ")).unwrap_or_else(|_| value.clone())
        })
        .collect();
    headers.remove(header::SET_COOKIE);
    for value in rewritten {
        headers.append(header::SET_COOKIE, value);
    }
}

fn upgrade_protocol(headers: &HeaderMap) -> Option<HeaderValue> {
    let requests_upgrade = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|t| t.trim().eq_ignore_ascii_case("upgrade"));
    if requests_upgrade {
        headers.get(header::UPGRADE).cloned()
    } else {
        None
    }
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    // Headers named in Connection are hop-by-hop too.
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|t| HeaderName::from_bytes(t.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in HOP_BY_HOP {
        headers.remove(name);
    }
}

fn strip_cookie(headers: &mut HeaderMap, name: &str) {
    let kept: Vec<String> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|cookie| {
            !cookie.is_empty() && cookie.split_once('=').map(|(key, _)| key.trim()) != Some(name)
        })
        .map(String::from)
        .collect();
    headers.remove(header::COOKIE);
    if !kept.is_empty()
        && let Ok(value) = HeaderValue::from_str(&kept.join("; "))
    {
        headers.insert(header::COOKIE, value);
    }
}

fn unreachable_page(port: u16) -> Response {
    let body = format!(
        "<!doctype html><html lang=en><meta charset=utf-8><title>Port {port}</title>\
         <body style=\"font:15px system-ui;background:#0f1115;color:#c9d1d9;display:grid;place-items:center;height:100vh;margin:0\">\
         <div style=\"text-align:center\"><h2 style=\"font-weight:600\">Nothing is listening on port {port}</h2>\
         <p style=\"color:#8b949e\">Start your server on <code>localhost:{port}</code> then reload the page.</p>\
         <p><button onclick=\"location.reload()\" style=\"font:inherit;padding:6px 14px;border-radius:6px;border:1px solid #30363d;background:#21262d;color:inherit;cursor:pointer\">Reload</button></p></div>"
    );
    let mut response = (StatusCode::BAD_GATEWAY, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_strip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; CF_Authorization=xyz; b=2"),
        );
        strip_cookie(&mut headers, JWT_COOKIE);
        assert_eq!(headers.get(header::COOKIE).unwrap(), "a=1; b=2");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("CF_Authorization=xyz"),
        );
        strip_cookie(&mut headers, JWT_COOKIE);
        assert!(headers.get(header::COOKIE).is_none());
    }

    #[test]
    fn cookie_domain_stripped() {
        let mut headers = HeaderMap::new();
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_static("sid=1; Domain=example.com; Path=/; HttpOnly"),
        );
        headers.append(header::SET_COOKIE, HeaderValue::from_static("a=2; Path=/"));
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_static("domain=3; DOMAIN = example.com; Secure"),
        );
        strip_cookie_domain(&mut headers);
        let cookies: Vec<&str> = headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert_eq!(
            cookies,
            ["sid=1; Path=/; HttpOnly", "a=2; Path=/", "domain=3; Secure"]
        );
    }

    #[test]
    fn hop_by_hop() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-foo"),
        );
        headers.insert("x-foo", HeaderValue::from_static("1"));
        headers.insert("x-bar", HeaderValue::from_static("1"));
        assert!(upgrade_protocol(&headers).is_none());
        strip_hop_by_hop(&mut headers);
        assert!(headers.get("x-foo").is_none() && headers.get(header::CONNECTION).is_none());
        assert!(headers.get("x-bar").is_some());
    }
}
