//! Embedded UI. The page is not cached; asset URLs include content hashes.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;
use serde::Deserialize;

use crate::App;

#[derive(RustEmbed)]
#[folder = "web/"]
struct Web;

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
                   img-src 'self' data:; connect-src 'self'; font-src 'self' data:; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

#[derive(Deserialize)]
struct AssetQuery {
    v: Option<String>,
}

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/", get(|| async { index() })).route(
        "/assets/{*path}",
        get(
            |Path(path): Path<String>, Query(query): Query<AssetQuery>, headers: HeaderMap| async move {
                asset(&path, query.v, &headers)
            },
        ),
    )
}

/// Short content hash of an embedded file.
fn version(path: &str) -> Option<String> {
    Web::get(path).map(|f| {
        f.metadata.sha256_hash()[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    })
}

/// Appends `?v=<hash>` to every `"/assets/<path>"` in the page.
fn versioned(html: &str) -> String {
    const PREFIX: &str = "\"/assets/";
    let mut result = String::with_capacity(html.len() + 512);
    let mut remaining = html;
    while let Some(i) = remaining.find(PREFIX) {
        let start = i + PREFIX.len();
        let Some(len) = remaining[start..].find('"') else {
            break;
        };
        let path = &remaining[start..start + len];
        result.push_str(&remaining[..start + len]);
        if !path.contains('?')
            && let Some(hash) = version(path)
        {
            result.push_str("?v=");
            result.push_str(&hash);
        }
        remaining = &remaining[start + len..];
    }
    result.push_str(remaining);
    result
}

fn index() -> Response {
    let Some(file) = Web::get("index.html") else {
        return (StatusCode::NOT_FOUND, "not found\n").into_response();
    };
    let html = versioned(&String::from_utf8_lossy(&file.data));
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    // Tiny, and the entry point to every versioned URL: never cache it.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    (headers, html).into_response()
}

fn asset(path: &str, requested_version: Option<String>, request_headers: &HeaderMap) -> Response {
    let (Some(file), Some(current_version)) = (Web::get(path), version(path)) else {
        return (StatusCode::NOT_FOUND, "not found\n").into_response();
    };
    let etag = format!("\"{current_version}\"");
    let mut headers = HeaderMap::new();
    headers.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        if requested_version.as_deref() == Some(current_version.as_str()) {
            // This URL can only ever mean these bytes.
            HeaderValue::from_static("public, max-age=31536000, immutable")
        } else {
            HeaderValue::from_static("no-cache")
        },
    );
    if etag_matches(request_headers, &current_version) {
        return (StatusCode::NOT_MODIFIED, headers).into_response();
    }
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref()).unwrap(),
    );
    (headers, file.data.into_owned()).into_response()
}

/// If-None-Match uses weak comparison (RFC 9110 §13.1.2) and may list
/// several tags; proxies such as Cloudflare weaken ETags (`W/"…"`) when they
/// compress a response.
fn etag_matches(request_headers: &HeaderMap, tag: &str) -> bool {
    let Some(value) = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    value.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate.trim_start_matches("W/").trim_matches('"') == tag
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_references_are_versioned() {
        let html = versioned(&String::from_utf8_lossy(
            &Web::get("index.html").unwrap().data,
        ));
        for asset in [
            "/assets/app.js",
            "/assets/style.css",
            "/assets/vendor/xterm.js",
            "/assets/vendor/codemirror.js",
        ] {
            let hash = version(asset.trim_start_matches("/assets/")).unwrap();
            assert!(
                html.contains(&format!("\"{asset}?v={hash}\"")),
                "{asset} not versioned"
            );
        }
        assert!(!html.contains("\"/assets/app.js\""));
    }

    #[test]
    fn weak_etags() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_static("W/\"abc\", \"def\""),
        );
        assert!(
            etag_matches(&headers, "abc")
                && etag_matches(&headers, "def")
                && !etag_matches(&headers, "xyz")
        );
    }
}
