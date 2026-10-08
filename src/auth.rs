//! Authentication via Cloudflare Access.
//!
//! cloudflared forwards every request that passed an Access policy with a
//! `Cf-Access-Jwt-Assertion` header. We verify that JWT ourselves (RS256
//! signature against the team's published keys, issuer, audience, expiry) so
//! that a request reaching the listener by any other path is rejected.
//! See https://developers.cloudflare.com/cloudflare-one/identity/authorization-cookie/validating-json/

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};

use crate::App;
use crate::config::AuthConfig;

pub const JWT_HEADER: &str = "cf-access-jwt-assertion";
pub const JWT_COOKIE: &str = "CF_Authorization";

/// The authenticated user, inserted as a request extension.
#[derive(Debug, Clone)]
pub struct User {
    pub email: String,
}

pub enum Auth {
    Access(Box<AccessVerifier>),
    Insecure(String),
}

impl Auth {
    pub fn new(config: &AuthConfig) -> Result<Self> {
        Ok(match config {
            AuthConfig::CloudflareAccess {
                team_domain,
                audiences,
            } => {
                if audiences.is_empty() {
                    bail!("auth.audiences must list at least one Access AUD tag");
                }
                Auth::Access(Box::new(AccessVerifier::new(
                    team_domain,
                    audiences.clone(),
                )?))
            }
            AuthConfig::Insecure { user } => Auth::Insecure(user.trim().to_lowercase()),
        })
    }

    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<User> {
        match self {
            Auth::Insecure(user) => Ok(User {
                email: user.clone(),
            }),
            Auth::Access(verifier) => {
                let token = headers
                    .get(JWT_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| anyhow!("missing {JWT_HEADER} header"))?;
                verifier.verify(token).await
            }
        }
    }

    /// Re-fetch Access signing keys (no-op in insecure mode).
    pub async fn refresh_now(&self) {
        if let Auth::Access(verifier) = self {
            verifier.refresh_now().await;
        }
    }
}

pub struct AccessVerifier {
    issuer: String,
    certs_url: String,
    audiences: Vec<String>,
    keys: RwLock<HashMap<String, Arc<DecodingKey>>>,
    last_fetch: Mutex<Option<Instant>>,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct Claims {
    email: Option<String>,
}

/// Don't hammer the certs endpoint when tokens with unknown `kid`s arrive.
const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

impl AccessVerifier {
    fn new(team_domain: &str, audiences: Vec<String>) -> Result<Self> {
        let team = team_domain
            .trim()
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        let host = if team.contains('.') {
            team
        } else {
            format!("{team}.cloudflareaccess.com")
        };
        Ok(Self {
            issuer: format!("https://{host}"),
            certs_url: format!("https://{host}/cdn-cgi/access/certs"),
            audiences,
            keys: RwLock::new(HashMap::new()),
            last_fetch: Mutex::new(None),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()?,
        })
    }

    /// Fetch the signing keys at startup so a misconfigured team domain fails
    /// loudly instead of on the first request.
    pub async fn prefetch(&self) -> Result<()> {
        self.refresh(true).await
    }

    /// Re-fetch the signing keys unconditionally, so keys Cloudflare has
    /// retired stop being accepted even if no token forces a refresh.
    pub async fn refresh_now(&self) {
        if let Err(e) = self.refresh(true).await {
            tracing::warn!("periodic Access key refresh failed: {e:#}");
        }
    }

    async fn verify(&self, token: &str) -> Result<User> {
        let header = jsonwebtoken::decode_header(token).context("malformed JWT")?;
        if header.alg != Algorithm::RS256 {
            bail!("unexpected JWT algorithm {:?}", header.alg);
        }
        let kid = header.kid.ok_or_else(|| anyhow!("JWT has no kid"))?;
        let key = self.key(&kid).await?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&self.audiences);
        validation.set_issuer(&[&self.issuer]);
        validation.set_required_spec_claims(&["exp", "iss", "aud"]);
        validation.validate_nbf = true;
        let decoded =
            jsonwebtoken::decode::<Claims>(token, &key, &validation).context("invalid JWT")?;
        let email = decoded
            .claims
            .email
            .filter(|e| !e.is_empty())
            .ok_or_else(|| anyhow!("JWT has no email (service tokens are not supported)"))?;
        Ok(User {
            email: email.to_lowercase(),
        })
    }

    async fn key(&self, kid: &str) -> Result<Arc<DecodingKey>> {
        if let Some(key) = self.keys.read().await.get(kid) {
            return Ok(key.clone());
        }
        // Unknown kid: Cloudflare rotates keys, so refresh once.
        self.refresh(false).await?;
        self.keys
            .read()
            .await
            .get(kid)
            .cloned()
            .ok_or_else(|| anyhow!("JWT signed with unknown key {kid}"))
    }

    async fn refresh(&self, force: bool) -> Result<()> {
        let mut last_fetch = self.last_fetch.lock().await;
        if !force && last_fetch.is_some_and(|t| t.elapsed() < MIN_REFRESH_INTERVAL) {
            return Ok(());
        }
        *last_fetch = Some(Instant::now());
        let key_set: JwkSet = self
            .http
            .get(&self.certs_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .with_context(|| format!("fetching {}", self.certs_url))?
            .json()
            .await
            .with_context(|| format!("decoding {}", self.certs_url))?;
        let mut keys = HashMap::new();
        for jwk in &key_set.keys {
            let Some(kid) = &jwk.common.key_id else {
                continue;
            };
            match DecodingKey::from_jwk(jwk) {
                Ok(key) => {
                    keys.insert(kid.clone(), Arc::new(key));
                }
                Err(e) => tracing::warn!("ignoring Access key {kid}: {e}"),
            }
        }
        if keys.is_empty() {
            bail!("{} returned no usable keys", self.certs_url);
        }
        tracing::debug!("loaded {} Cloudflare Access keys", keys.len());
        *self.keys.write().await = keys;
        Ok(())
    }
}

/// Authenticates every request; inserts [`User`] on success.
pub async fn require_user(State(app): State<Arc<App>>, mut req: Request, next: Next) -> Response {
    match app.auth.authenticate(req.headers()).await {
        Ok(user) => {
            req.extensions_mut().insert(user);
            next.run(req).await
        }
        Err(e) => {
            tracing::warn!("rejected {} {}: {e:#}", req.method(), req.uri().path());
            (
                StatusCode::FORBIDDEN,
                "forbidden: Cloudflare Access authentication required\n",
            )
                .into_response()
        }
    }
}

/// Rejects cross-site requests. State-changing requests and WebSocket
/// upgrades must be same-origin (`Origin` == `Host`, so a missing `Origin` is
/// rejected too): this blocks CSRF and cross-site WebSocket hijacking. GET
/// reads are additionally refused when the browser labels them cross-site via
/// `Sec-Fetch-Site`, so another site cannot pull an API response into an
/// `<img>`/`<script>` or make us do expensive work (a download, a search).
pub async fn same_origin(req: Request, next: Next) -> Response {
    let strict = !matches!(req.method().as_str(), "GET" | "HEAD" | "OPTIONS")
        || req.headers().contains_key(axum::http::header::UPGRADE);
    let reject = if strict {
        !is_same_origin(req.headers())
    } else {
        // GET: trust the browser's Fetch Metadata when present. "same-origin"
        // and "none" (a direct navigation) are fine; "same-site"/"cross-site"
        // are not. Absent (old client, curl) → allowed, as there is no cookie.
        matches!(
            sec_fetch_site(req.headers()),
            Some("cross-site") | Some("same-site")
        )
    };
    if reject {
        tracing::warn!(
            "rejected cross-origin {} {}",
            req.method(),
            req.uri().path()
        );
        return (StatusCode::FORBIDDEN, "forbidden: cross-origin request\n").into_response();
    }
    next.run(req).await
}

fn sec_fetch_site(headers: &HeaderMap) -> Option<&str> {
    headers.get("sec-fetch-site").and_then(|v| v.to_str().ok())
}

fn is_same_origin(headers: &HeaderMap) -> bool {
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok());
    let (Some(origin), Some(host)) = (origin, host) else {
        return false;
    };
    let origin_host = origin.split_once("://").map(|(_, h)| h).unwrap_or("");
    !origin_host.is_empty() && origin_host.eq_ignore_ascii_case(host)
}

#[cfg(test)]
pub(crate) async fn test_auth() -> Auth {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let verifier = AccessVerifier::new("tests", vec!["codeenv-tests".into()]).unwrap();
    let key = DecodingKey::from_rsa_pem(include_bytes!("testdata/access_test_pub.pem")).unwrap();
    verifier
        .keys
        .write()
        .await
        .insert("test".into(), Arc::new(key));
    *verifier.last_fetch.lock().await = Some(Instant::now());
    Auth::Access(Box::new(verifier))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn token(kid: &str, iss: &str, aud: &str, exp: u64, email: Option<&str>) -> String {
        let mut h = jsonwebtoken::Header::new(Algorithm::RS256);
        h.kid = Some(kid.into());
        let mut claims = serde_json::json!({ "iss": iss, "aud": [aud], "exp": exp, "iat": now() });
        if let Some(e) = email {
            claims["email"] = e.into();
        }
        let key =
            jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("testdata/access_test_key.pem"))
                .unwrap();
        jsonwebtoken::encode(&h, &claims, &key).unwrap()
    }

    #[tokio::test]
    async fn access_jwt() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
        let verifier = AccessVerifier::new("myteam", vec!["aud1".into(), "aud2".into()]).unwrap();
        assert_eq!(verifier.issuer, "https://myteam.cloudflareaccess.com");
        let public_key =
            DecodingKey::from_rsa_pem(include_bytes!("testdata/access_test_pub.pem")).unwrap();
        verifier
            .keys
            .write()
            .await
            .insert("k1".into(), Arc::new(public_key));
        // Keys are present: no fetch must happen for k1.
        *verifier.last_fetch.lock().await = Some(Instant::now());

        let issuer = "https://myteam.cloudflareaccess.com";
        let expires = now() + 600;
        let user = verifier
            .verify(&token(
                "k1",
                issuer,
                "aud2",
                expires,
                Some("Me@Example.com"),
            ))
            .await
            .unwrap();
        assert_eq!(user.email, "me@example.com");

        assert!(
            verifier
                .verify(&token("k1", issuer, "other-aud", expires, Some("a@b.c")))
                .await
                .is_err()
        );
        assert!(
            verifier
                .verify(&token(
                    "k1",
                    "https://evil.cloudflareaccess.com",
                    "aud1",
                    expires,
                    Some("a@b.c")
                ))
                .await
                .is_err()
        );
        assert!(
            verifier
                .verify(&token("k1", issuer, "aud1", now() - 3600, Some("a@b.c")))
                .await
                .is_err()
        );
        assert!(
            verifier
                .verify(&token("k1", issuer, "aud1", expires, None))
                .await
                .is_err()
        );
        // Unknown kid within the refresh back-off: rejected without fetching.
        assert!(
            verifier
                .verify(&token("k2", issuer, "aud1", expires, Some("a@b.c")))
                .await
                .is_err()
        );
        // Tampered payload.
        let token_text = token("k1", issuer, "aud1", expires, Some("a@b.c"));
        let mut parts: Vec<&str> = token_text.split('.').collect();
        let forged = token("k1", issuer, "aud1", expires, Some("admin@b.c"));
        parts[1] = forged.split('.').nth(1).unwrap();
        let tampered = format!(
            "{}.{}.{}",
            parts[0],
            parts[1],
            token_text.split('.').nth(2).unwrap()
        );
        assert!(verifier.verify(&tampered).await.is_err());
        // alg=none / HS256 tokens are refused before any key lookup.
        let mut header = jsonwebtoken::Header::new(Algorithm::HS256);
        header.kid = Some("k1".into());
        let hmac_token = jsonwebtoken::encode(
            &header,
            &serde_json::json!({"iss": issuer, "aud": "aud1", "exp": expires, "email": "a@b.c"}),
            &jsonwebtoken::EncodingKey::from_secret(b"x"),
        )
        .unwrap();
        assert!(verifier.verify(&hmac_token).await.is_err());
    }

    #[test]
    fn origin_check() {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("vps1.example.com"));
        assert!(!is_same_origin(&headers));
        headers.insert(
            "origin",
            HeaderValue::from_static("https://vps1.example.com"),
        );
        assert!(is_same_origin(&headers));
        headers.insert(
            "origin",
            HeaderValue::from_static("https://p3000-vps1.example.com"),
        );
        assert!(!is_same_origin(&headers));
        headers.insert("origin", HeaderValue::from_static("null"));
        assert!(!is_same_origin(&headers));
    }
}
