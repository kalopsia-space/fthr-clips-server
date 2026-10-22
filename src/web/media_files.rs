use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::app::AppState;
use crate::db::{jobs, media};
use crate::media::paths::DerivedPaths;
use crate::web::security::AllowFraming;

const DERIVED_CACHE: &str = "public, max-age=3600";
const PRIVATE_CACHE: &str = "private, max-age=0";

/// `GET /media/{slug}/{file}`. Only paths recorded in the DB or derived from
/// the media id are ever opened; nothing from the URL becomes a path.
pub async fn get_media_file(
    State(state): State<AppState>,
    UrlPath((slug, file)): UrlPath<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    match serve(
        &state,
        &slug,
        &file,
        q.get("download").is_some_and(|v| v == "1"),
        &headers,
    )
    .await
    {
        Some(r) => r,
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve(
    state: &AppState,
    slug: &str,
    file: &str,
    download: bool,
    req_headers: &HeaderMap,
) -> Option<Response> {
    let m = media::get_by_slug(&state.db, slug).await.ok()??;
    if !m.available {
        return None;
    }
    let derived = DerivedPaths::new(&state.config.data_dir, &m.id);
    let is_original = file.starts_with("original.");
    let (path, mime, cache) = if is_original {
        if file["original.".len()..] != m.extension {
            return None;
        }
        let mime = mime_guess::from_ext(&m.extension)
            .first()
            .map(|m| m.essence_str().to_string())
            .unwrap_or_else(|| "application/octet-stream".into());
        let cache = if m.public {
            DERIVED_CACHE
        } else {
            PRIVATE_CACHE
        };
        (PathBuf::from(&m.path), mime, cache)
    } else {
        let (path, mime) = match file {
            "poster.jpg" => (derived.poster(), "image/jpeg"),
            "thumb.jpg" => (derived.thumb(), "image/jpeg"),
            "preview.mp4" => (derived.preview(), "video/mp4"),
            _ => {
                let h: u32 = match file {
                    "480p.mp4" => 480,
                    "720p.mp4" => 720,
                    "1080p.mp4" => 1080,
                    _ => return None,
                };
                let ready = jobs::renditions_for(&state.db, &m.id)
                    .await
                    .ok()?
                    .iter()
                    .any(|r| r.height == i64::from(h) && r.state == "ready");
                if !ready {
                    return None;
                }
                (derived.rendition(h), "video/mp4")
            }
        };
        (path, mime.to_string(), DERIVED_CACHE)
    };

    let canonical = tokio::fs::canonicalize(&path).await.ok()?;
    if !within_roots(state, &canonical).await {
        tracing::warn!(slug, path = %canonical.display(), "media path outside allowed roots");
        return None;
    }

    let mime: mime_guess::mime::Mime = mime.parse().ok()?;
    let mut req = Request::builder().method("GET").uri("/");
    for name in [
        header::RANGE,
        header::IF_RANGE,
        header::IF_NONE_MATCH,
        header::IF_MODIFIED_SINCE,
    ] {
        if let Some(v) = req_headers.get(&name) {
            req = req.header(name, v);
        }
    }
    let req = req.body(Body::empty()).ok()?;
    let resp = ServeFile::new_with_mime(&canonical, &mime)
        .oneshot(req)
        .await
        .ok()?;
    if resp.status() == StatusCode::NOT_FOUND {
        return None;
    }
    let ok = matches!(
        resp.status(),
        StatusCode::OK | StatusCode::PARTIAL_CONTENT | StatusCode::NOT_MODIFIED
    );
    let mut resp = resp.map(Body::new);
    if ok {
        resp.extensions_mut().insert(AllowFraming);
    }
    let h = resp.headers_mut();
    if ok {
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    }
    if is_original {
        let name = Path::new(&m.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("original.{}", m.extension));
        if let Ok(v) = HeaderValue::from_str(&content_disposition(&name, download)) {
            h.insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Some(resp)
}

async fn within_roots(state: &AppState, canonical: &Path) -> bool {
    let cfg = &state.config;
    let roots = std::iter::once(&cfg.data_dir)
        .chain(cfg.video_dirs.iter())
        .chain(cfg.image_dirs.iter());
    for root in roots {
        if let Ok(r) = tokio::fs::canonicalize(root).await {
            if canonical.starts_with(&r) {
                return true;
            }
        }
    }
    false
}

fn content_disposition(name: &str, download: bool) -> String {
    let ascii: String = name
        .chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    let kind = if download { "attachment" } else { "inline" };
    format!(
        "{kind}; filename=\"{ascii}\"; filename*=UTF-8''{}",
        urlencoding::encode(name)
    )
}
