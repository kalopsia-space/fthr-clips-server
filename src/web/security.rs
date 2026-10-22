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
const CSP_FRAMEABLE: &str = "default-src 'self'; img-src 'self' data:; media-src 'self'; style-src 'self'; script-src 'self'; font-src 'self'; frame-ancestors *";

/// Marker a handler inserts into its response extensions
/// (`response.extensions_mut().insert(AllowFraming)`) to opt out of
/// `X-Frame-Options: DENY`. `security_headers` checks for it after the
/// handler runs, then also omits `X-Frame-Options` and emits the CSP with
/// `frame-ancestors *` instead of `'none'`.
#[derive(Debug, Clone, Copy)]
pub struct AllowFraming;

/// True when `peer` falls inside one of the trusted proxy networks.
pub fn is_trusted_peer(peer: IpAddr, trusted: &[IpNet]) -> bool {
    trusted.iter().any(|n| n.contains(&peer))
}

/// First value of `X-Forwarded-Proto`, trimmed and lowercased. Callers must
/// only trust it when the peer is a trusted proxy.
pub fn forwarded_proto(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_ascii_lowercase())
}

/// True when the request arrived over HTTPS, as reported by a trusted proxy.
pub fn request_is_https(headers: &HeaderMap, peer: IpAddr, trusted: &[IpNet]) -> bool {
    is_trusted_peer(peer, trusted) && forwarded_proto(headers).is_some_and(|v| v == "https")
}

fn parse_hop(s: &str) -> Option<IpAddr> {
    let s = s.trim();
    s.parse::<IpAddr>()
        .ok()
        .or_else(|| s.parse::<SocketAddr>().ok().map(|a| a.ip()))
        .or_else(|| {
            s.strip_prefix('[')
                .and_then(|r| r.strip_suffix(']'))
                .and_then(|r| r.parse().ok())
        })
}

/// Resolve the real client address. `X-Forwarded-For` is only consulted when
/// the direct peer is a trusted proxy; the rightmost entry that is not itself
/// a trusted proxy wins.
pub fn client_ip(headers: &HeaderMap, peer: IpAddr, trusted: &[IpNet]) -> IpAddr {
    if !is_trusted_peer(peer, trusted) {
        return peer;
    }
    for value in headers.get_all("x-forwarded-for").iter().rev() {
        let Ok(value) = value.to_str() else {
            return peer;
        };
        for hop in value.rsplit(',') {
            // Fail closed: an unparseable hop may be client-forged, so do
            // not keep walking left past it.
            let Some(addr) = parse_hop(hop) else {
                return peer;
            };
            if !is_trusted_peer(addr, trusted) {
                return addr;
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
    state: Mutex<LimiterState>,
}

struct LimiterState {
    hits: HashMap<String, VecDeque<Instant>>,
    last_sweep: Instant,
}

impl RateLimiter {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit: limit as usize,
            window,
            state: Mutex::new(LimiterState {
                hits: HashMap::new(),
                last_sweep: Instant::now(),
            }),
        }
    }

    /// Returns true and records a hit if `key` is under the limit.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let window = self.window;
        let prune = |q: &mut VecDeque<Instant>| {
            while q.front().is_some_and(|t| now.duration_since(*t) >= window) {
                q.pop_front();
            }
        };
        // Sweep the whole map at most once per window so it cannot grow
        // without bound, without scanning every key on every call.
        if now.duration_since(st.last_sweep) >= window {
            st.hits.retain(|_, q| {
                prune(q);
                !q.is_empty()
            });
            st.last_sweep = now;
        }
        let q = st.hits.entry(key.to_string()).or_default();
        prune(q);
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
    let set = |h: &mut HeaderMap, name: &'static str, value: &'static str| {
        h.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    };
    set(h, "x-content-type-options", "nosniff");
    set(h, "referrer-policy", "no-referrer");
    if allow_framing {
        // frame-ancestors would override a missing X-Frame-Options, so it
        // must be relaxed too.
        set(h, "content-security-policy", CSP_FRAMEABLE);
        h.remove("x-frame-options");
    } else {
        set(h, "content-security-policy", CSP);
        set(h, "x-frame-options", "DENY");
    }
    if https {
        set(h, "strict-transport-security", "max-age=31536000");
    }
    resp
}

/// Test support: a router with one route that opts out of framing.
#[doc(hidden)]
pub fn test_router_with_allow_framing(state: AppState) -> axum::Router {
    use axum::response::IntoResponse;
    axum::Router::new()
        .route(
            "/framed",
            axum::routing::get(|| async {
                let mut r = "ok".into_response();
                r.extensions_mut().insert(AllowFraming);
                r
            }),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            security_headers,
        ))
        .with_state(state)
}
