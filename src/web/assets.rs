//! Embedded static files (`static/`), served with long-lived caching. Templates
//! add `?v={asset_v()}` so a rebuild with changed assets busts caches.

use std::sync::OnceLock;

use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use xxhash_rust::xxh3::Xxh3;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

/// First 8 hex characters of a hash over every embedded file (path and
/// contents, in path order). Computed once.
pub fn asset_v() -> &'static str {
    static V: OnceLock<String> = OnceLock::new();
    V.get_or_init(|| {
        let mut names: Vec<_> = Assets::iter().collect();
        names.sort();
        let mut h = Xxh3::new();
        for name in names {
            h.update(name.as_bytes());
            if let Some(f) = Assets::get(&name) {
                h.update(&f.data);
            }
        }
        format!("{:016x}", h.digest())[..8].to_string()
    })
}

/// `GET /static/{*path}`.
pub async fn serve_static(Path(path): Path<String>) -> Response {
    let Some(file) = Assets::get(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    let mut resp = file.data.into_owned().into_response();
    let h = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime.essence_str()) {
        let ct = if mime.type_() == mime_guess::mime::TEXT {
            HeaderValue::from_str(&format!("{}; charset=utf-8", mime.essence_str())).unwrap_or(v)
        } else {
            v
        };
        h.insert(header::CONTENT_TYPE, ct);
    }
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    resp
}
