//! Security primitives: response headers, rate limiting, client IP
//! resolution behind trusted proxies, and CSRF checking.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use ipnet::IpNet;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::app::AppState;
use crate::db::sessions::Session;

const CSP: &str = "default-src 'self'; img-src 'self' data:; media-src 'self'; style-src 'self'; script-src 'self'; font-src 'self'; frame-ancestors 'none'";

/// Marker a handler inserts into its response extensions
/// (`response.extensions_mut().insert(AllowFraming)`) to opt out of
/// `X-Frame-Options: DENY`. `security_headers` checks for it after the
/// handler runs.
#[derive(Debug, Clone, Copy)]
pub struct AllowFraming;

/// True when `peer` falls inside one of the trusted proxy networks.
pub fn is_trusted_peer(peer: IpAddr, trusted: &[IpNet]) -> bool {
    trusted.iter().any(|n| n.contains(&peer))
}

/// True when the request arrived over HTTPS, as reported by a trusted proxy.
pub fn request_is_https(headers: &HeaderMap, peer: IpAddr, trusted: &[IpNet]) -> bool {
    is_trusted_peer(peer, trusted)
        && headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("https"))
}

fn parse_hop(s: &str) -> Option<IpAddr> {
    let s = s.trim().trim_start_matches('[').trim_end_matches(']');
    s.parse().ok()
}

/// Resolve the real client address. `X-Forwarded-For` is only consulted when
/// the direct peer is a trusted proxy; the rightmost entry that is not itself
/// a trusted proxy wins.
pub fn client_ip(headers: &HeaderMap, peer: IpAddr, trusted: &[IpNet]) -> IpAddr {
    if !is_trusted_peer(peer, trusted) {
        return peer;
    }
    for value in headers.get_all("x-forwarded-for").iter().rev() {
        let Ok(value) = value.to_str() else { continue };
        for hop in value.rsplit(',') {
            if let Some(addr) = parse_hop(hop) {
                if !is_trusted_peer(addr, trusted) {
                    return addr;
                }
            }
        }
    }
    peer
}

/// Hex SHA-256 of `secret || ip`, for storing client identity without the IP.
pub fn ip_hash(secret: &[u8; 32], ip: IpAddr) -> String {
    let mut h = Sha256::new();
    h.update(secret);
    h.update(ip.to_string().as_bytes());
    hex::encode(h.finalize())
}

/// Constant-time CSRF check against the session token, taken from the
/// `X-CSRF-Token` header or else the form field.
pub fn csrf_ok(session: &Session, headers: &HeaderMap, form_token: Option<&str>) -> bool {
    let supplied = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .or(form_token);
    match supplied {
        Some(t) => t.as_bytes().ct_eq(session.csrf.as_bytes()).into(),
        None => false,
    }
}

/// In-memory sliding-window rate limiter keyed by string.
pub struct RateLimiter {
    limit: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit: limit as usize,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true and records a hit if `key` is under the limit.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut map = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        // Drop expired keys so the map cannot grow without bound.
        map.retain(|_, q| {
            while q
                .front()
                .is_some_and(|t| now.duration_since(*t) >= self.window)
            {
                q.pop_front();
            }
            !q.is_empty()
        });
        let q = map.entry(key.to_string()).or_default();
        if q.len() >= self.limit {
            return false;
        }
        q.push_back(now);
        true
    }
}

/// Middleware adding the standard security headers to every response.
pub async fn security_headers(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let https = request_is_https(req.headers(), peer.ip(), &state.config.trusted_proxies);
    let mut resp = next.run(req).await;
    let allow_framing = resp.extensions().get::<AllowFraming>().is_some();
    let h = resp.headers_mut();
    let mut set = |name: &'static str, value: &'static str| {
        h.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    };
    set("x-content-type-options", "nosniff");
    set("referrer-policy", "no-referrer");
    set("content-security-policy", CSP);
    if !allow_framing {
        set("x-frame-options", "DENY");
    }
    if https {
        set("strict-transport-security", "max-age=31536000");
    }
    resp
}
