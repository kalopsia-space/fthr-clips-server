mod common;

use common::TestServer;
use serde_json::{json, Value};

const PW: &str = "correct horse";

async fn server() -> TestServer {
    TestServer::spawn(&[("ADMIN_PASSWORD", PW)]).await
}

async fn login_json(s: &TestServer, c: &reqwest::Client, pw: &str) -> reqwest::Response {
    c.post(s.url("/admin/login"))
        .json(&json!({ "password": pw }))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn login_sets_cookie_and_dashboard_is_gated() {
    let s = server().await;
    let c = s.client_with_cookies();
    let r = c.get(s.url("/admin")).send().await.unwrap();
    assert_eq!(r.status(), 303);
    assert_eq!(r.headers()["location"], "/admin/login?next=%2Fadmin");
    let r = c.get(s.url("/admin/api/session")).send().await.unwrap();
    assert_eq!(r.status(), 401);
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "unauthorized");

    let r = login_json(&s, &c, PW).await;
    assert_eq!(r.status(), 200);
    let set = r.headers()["set-cookie"].to_str().unwrap().to_string();
    let csrf = r.json::<Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(set.starts_with("fthr_admin="), "{set}");
    let r = c.get(s.url("/admin/files")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let r = c.get(s.url("/admin/api/session")).send().await.unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["csrf"], csrf);

    // Public pages reflect the session.
    let body = c
        .get(s.url("/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("action=\"/admin/logout\""));
    assert!(body.contains(&csrf));
}

#[tokio::test]
async fn wrong_password_is_401() {
    let s = server().await;
    let c = s.client_with_cookies();
    let r = login_json(&s, &c, "nope").await;
    assert_eq!(r.status(), 401);
    assert!(r.headers().get("set-cookie").is_none());
    assert_eq!(r.json::<Value>().await.unwrap()["error"], "invalid");
    let r = c
        .post(s.url("/admin/login"))
        .form(&[("password", "nope")])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 303);
    assert_eq!(r.headers()["location"], "/admin/login?error=1");
}

#[tokio::test]
async fn no_password_configured_always_401() {
    let s = TestServer::spawn(&[]).await;
    let c = s.client_with_cookies();
    assert_eq!(login_json(&s, &c, "").await.status(), 401);
    assert_eq!(login_json(&s, &c, "anything").await.status(), 401);
}

#[tokio::test]
async fn login_rate_limited_429() {
    let s = server().await;
    let c = s.client_with_cookies();
    let mut last = 0;
    for _ in 0..11 {
        last = login_json(&s, &c, "wrong").await.status().as_u16();
    }
    assert_eq!(last, 429);
}

#[tokio::test]
async fn logout_invalidates_session() {
    let s = server().await;
    let c = s.client_with_cookies();
    let csrf = login_json(&s, &c, PW).await.json::<Value>().await.unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_string();
    let r = c
        .post(s.url("/admin/logout"))
        .header("x-csrf-token", &csrf)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(r.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    let r = c.get(s.url("/admin/api/session")).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn csrf_required_on_logout() {
    let s = server().await;
    let c = s.client_with_cookies();
    login_json(&s, &c, PW).await;
    let r = c.post(s.url("/admin/logout")).send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = c
        .post(s.url("/admin/logout"))
        .header("x-csrf-token", "bogus")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    // Still signed in.
    assert_eq!(
        c.get(s.url("/admin/api/session"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn cookie_not_secure_over_http() {
    let s = server().await;
    let c = s.client_with_cookies();
    let r = login_json(&s, &c, PW).await;
    let set = r.headers()["set-cookie"].to_str().unwrap();
    assert!(set.contains("HttpOnly"), "{set}");
    assert!(set.contains("SameSite=Lax"), "{set}");
    assert!(set.contains("Path=/"), "{set}");
    assert!(set.contains("Max-Age=28800"), "{set}");
    assert!(!set.contains("Secure"), "{set}");
}

#[tokio::test]
async fn hashed_password_and_safe_next() {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let hash = argon2::Argon2::default()
        .hash_password(PW.as_bytes(), &salt)
        .unwrap()
        .to_string();
    let s = TestServer::spawn(&[("ADMIN_PASSWORD_HASH", &hash)]).await;
    let c = s.client_with_cookies();
    assert_eq!(login_json(&s, &c, "bad").await.status(), 401);
    let r = c
        .post(s.url("/admin/login"))
        .form(&[("password", PW), ("next", "//evil.example")])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 303);
    assert_eq!(r.headers()["location"], "/admin");
}

#[tokio::test]
async fn next_is_strictly_validated() {
    let s = server().await;
    let cases = [
        ("/\t/evil.com", "/admin"),
        ("/\\evil", "/admin"),
        ("/\u{7f}x", "/admin"),
        ("https://evil.com/", "/admin"),
        ("/admin/files?page=2", "/admin/files?page=2"),
    ];
    for (next, want) in cases {
        let c = s.client_with_cookies();
        let r = c
            .post(s.url("/admin/login"))
            .form(&[("password", PW), ("next", next)])
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 303);
        assert_eq!(r.headers()["location"], want, "next={next:?}");
    }
}

#[tokio::test]
async fn blocked_ip_does_not_consume_global_budget() {
    let s = server().await;
    // Exhaust this client's per-IP budget, then keep hammering.
    let c = s.client_with_cookies();
    for _ in 0..30 {
        login_json(&s, &c, "wrong").await;
    }
    // Per-IP and global both cap at 10 here (single IP), so the global bucket
    // must not have been charged beyond 10: it just must not panic and stay 429.
    assert_eq!(login_json(&s, &c, PW).await.status(), 429);
}
