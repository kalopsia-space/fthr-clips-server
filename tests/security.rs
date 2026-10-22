mod common;

use std::net::IpAddr;
use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue};
use chrono::Utc;
use fthr_clips_server::db::sessions::Session;
use fthr_clips_server::web::security::{
    client_ip, csrf_ok, ip_hash, request_is_https, RateLimiter,
};
use ipnet::IpNet;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn net(s: &str) -> IpNet {
    s.parse().unwrap()
}

fn hm(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in pairs {
        h.insert(*k, HeaderValue::from_str(v).unwrap());
    }
    h
}

#[tokio::test]
async fn headers_present_on_healthz() {
    let s = common::TestServer::spawn(&[("UPLOAD_TOKEN", "t")]).await;
    let r = s.client.get(s.url("/healthz")).send().await.unwrap();
    let h = r.headers();
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(
        h["content-security-policy"],
        "default-src 'self'; img-src 'self' data:; media-src 'self'; style-src 'self'; script-src 'self'; font-src 'self'; frame-ancestors 'none'"
    );
    assert_eq!(h.get_all("content-security-policy").iter().count(), 1);
    assert!(h.get("strict-transport-security").is_none());
}

#[test]
fn rate_limiter_blocks_after_limit() {
    let rl = RateLimiter::new(2, Duration::from_millis(100));
    assert!(rl.check("a"));
    assert!(rl.check("a"));
    assert!(!rl.check("a"));
    assert!(rl.check("b"));
    std::thread::sleep(Duration::from_millis(150));
    assert!(rl.check("a"));
}

#[test]
fn client_ip_ignores_xff_when_peer_untrusted() {
    let h = hm(&[("x-forwarded-for", "1.2.3.4")]);
    let trusted = [net("10.0.0.0/8")];
    assert_eq!(
        client_ip(&h, ip("203.0.113.5"), &trusted),
        ip("203.0.113.5")
    );
}

#[test]
fn client_ip_uses_rightmost_untrusted_hop_when_peer_trusted() {
    let trusted = [net("10.0.0.0/8")];
    let h = hm(&[("x-forwarded-for", "9.9.9.9, 198.51.100.7 , 10.1.1.1")]);
    assert_eq!(client_ip(&h, ip("10.0.0.2"), &trusted), ip("198.51.100.7"));
    let h = hm(&[("x-forwarded-for", "10.1.1.1, [2001:db8::1]")]);
    assert_eq!(client_ip(&h, ip("10.0.0.2"), &trusted), ip("2001:db8::1"));
    let h = hm(&[("x-forwarded-for", "10.1.1.1")]);
    assert_eq!(client_ip(&h, ip("10.0.0.2"), &trusted), ip("10.0.0.2"));
}

#[test]
fn request_is_https_ignores_forged_proto() {
    let h = hm(&[("x-forwarded-proto", "https")]);
    let peer = ip("203.0.113.5");
    assert!(!request_is_https(&h, peer, &[]));
    assert!(request_is_https(&h, peer, &[net("203.0.113.0/24")]));
}

#[test]
fn ip_hash_and_csrf() {
    let a = ip_hash(&[1u8; 32], ip("1.2.3.4"));
    assert_eq!(a.len(), 64);
    assert_ne!(a, ip_hash(&[2u8; 32], ip("1.2.3.4")));
    let s = Session {
        id: "i".into(),
        csrf: "tok".into(),
        created_at: Utc::now(),
        expires_at: Utc::now(),
    };
    assert!(csrf_ok(&s, &hm(&[("x-csrf-token", "tok")]), None));
    assert!(csrf_ok(&s, &HeaderMap::new(), Some("tok")));
    assert!(!csrf_ok(&s, &hm(&[("x-csrf-token", "bad")]), Some("tok")));
    assert!(!csrf_ok(&s, &HeaderMap::new(), None));
}

#[test]
fn client_ip_fails_closed_on_unparseable_hops() {
    let trusted = [net("10.0.0.0/8")];
    let peer = ip("10.0.0.2");
    let h = hm(&[("x-forwarded-for", "203.0.113.9:4444")]);
    assert_eq!(client_ip(&h, peer, &trusted), ip("203.0.113.9"));
    let h = hm(&[("x-forwarded-for", "[2001:db8::1]:443")]);
    assert_eq!(client_ip(&h, peer, &trusted), ip("2001:db8::1"));
    let h = hm(&[("x-forwarded-for", "203.0.113.9, garbage")]);
    assert_eq!(client_ip(&h, peer, &trusted), peer);
}

#[tokio::test]
async fn headers_present_on_404() {
    let s = common::TestServer::spawn(&[("UPLOAD_TOKEN", "t")]).await;
    let r = s.client.get(s.url("/no-such-path")).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let h = r.headers();
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["x-frame-options"], "DENY");
    assert!(h["content-security-policy"]
        .to_str()
        .unwrap()
        .ends_with("frame-ancestors 'none'"));
}

#[tokio::test]
async fn allow_framing_relaxes_frame_headers() {
    use fthr_clips_server::app::AppState;
    use fthr_clips_server::config::Config;
    use fthr_clips_server::db::Db;
    use fthr_clips_server::settings::SettingsStore;
    use std::sync::Arc;
    let dir = tempfile::TempDir::new().unwrap();
    let mut vars = std::collections::HashMap::new();
    vars.insert(
        "DATA_DIR".to_string(),
        dir.path().to_string_lossy().into_owned(),
    );
    let config = Config::from_env_map(&vars, None).unwrap();
    let db = Db::open(&config.data_dir.join("library.db")).await.unwrap();
    let settings = Arc::new(SettingsStore::load(&db, &config).await.unwrap());
    let state = AppState::new(config, db, settings);
    let app = fthr_clips_server::web::security::test_router_with_allow_framing(state);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            l,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let r = reqwest::get(format!("http://{addr}/framed")).await.unwrap();
    assert!(r.headers().get("x-frame-options").is_none());
    assert!(r.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .ends_with("frame-ancestors *"));
}
