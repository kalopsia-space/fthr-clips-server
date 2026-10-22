//! Clip (`/w/{slug}`) and image (`/i/{slug}`) pages: playback sources,
//! OpenGraph tags and view counting.

use std::collections::HashMap;
use std::net::SocketAddr;

use askama::Template;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::app::AppState;
use crate::db::jobs::{self, Rendition};
use crate::db::media::{self, Media};
use crate::media::kind::MediaKind;
use crate::media::playable::is_browser_playable;
use crate::settings::public_base;
use crate::web::admin::OptionalAdmin;
use crate::web::assets::asset_v;
use crate::web::library::{
    fmt_duration, fmt_relative, fmt_resolution, fmt_size, fmt_views, media_url, page_path, render,
    when, PageError, TagView, PLACEHOLDER_URL,
};
use crate::web::security::{client_ip, ip_hash, request_is_https, AllowFraming};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackSource {
    pub url: String,
    pub label: String,
    pub mime: String,
}

/// ffprobe reports compound format names (`mov,mp4,m4a,3gp,3g2,mj2`,
/// `matroska,webm`); reduce them to one token the playability check knows.
fn normalise_container(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    let parts: Vec<&str> = lower.split(',').map(str::trim).collect();
    for known in ["mp4", "webm", "mov", "m4v"] {
        if parts.contains(&known) {
            return known.to_string();
        }
    }
    match parts.first() {
        Some(&"matroska") => "mkv".to_string(),
        Some(p) => (*p).to_string(),
        None => String::new(),
    }
}

pub fn original_playable(m: &Media) -> bool {
    let container = normalise_container(m.container.as_deref().unwrap_or(&m.extension));
    let v = m.codec_video.as_deref().map(str::to_ascii_lowercase);
    let a = m.codec_audio.as_deref().map(str::to_ascii_lowercase);
    if v.is_none() && a.is_none() {
        // Not probed yet: trust the container so fresh uploads play at once.
        return matches!(container.as_str(), "mp4" | "webm" | "mov" | "m4v");
    }
    is_browser_playable(&container, v.as_deref(), a.as_deref())
}

/// Sources best-first: the original when the browser can play it, then ready
/// renditions from tallest to shortest.
pub fn playback_sources(m: &Media, renditions: &[Rendition]) -> Vec<PlaybackSource> {
    let mut out = Vec::new();
    if m.kind == MediaKind::Video && original_playable(m) {
        let mime = mime_guess::from_ext(&m.extension)
            .first()
            .map(|x| x.essence_str().to_string())
            .unwrap_or_else(|| "video/mp4".into());
        out.push(PlaybackSource {
            url: media_url(m, &format!("original.{}", m.extension)),
            label: "Source".into(),
            mime,
        });
    }
    let mut ready: Vec<&Rendition> = renditions
        .iter()
        .filter(|r| r.state == "ready" && matches!(r.height, 480 | 720 | 1080))
        .collect();
    ready.sort_by_key(|r| std::cmp::Reverse(r.height));
    for r in ready {
        out.push(PlaybackSource {
            url: media_url(m, &format!("{}p.mp4", r.height)),
            label: format!("{}p", r.height),
            mime: "video/mp4".into(),
        });
    }
    out
}

/// One row of the Quality panel.
#[derive(Debug, Clone)]
pub struct QualityRow {
    pub height: u32,
    /// "none" | "queued" | "running" | "ready" | "failed"
    pub state: String,
    pub pct: u32,
    pub error: String,
}

fn quality_rows(ladder: &[u32], renditions: &[Rendition], signed_in: bool) -> Vec<QualityRow> {
    let mut heights: Vec<u32> = renditions.iter().map(|r| r.height as u32).collect();
    if signed_in {
        heights.extend(ladder.iter().copied());
    }
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    heights
        .into_iter()
        .map(|h| {
            let r = renditions.iter().find(|r| r.height == i64::from(h));
            QualityRow {
                height: h,
                state: r.map_or("none".into(), |r| r.state.clone()),
                pct: r.map_or(0, |r| (r.progress.clamp(0.0, 1.0) * 100.0).round() as u32),
                error: r.and_then(|r| r.error.clone()).unwrap_or_default(),
            }
        })
        .collect()
}

pub fn og_video(sources: &[PlaybackSource]) -> Option<&PlaybackSource> {
    sources.first()
}

#[derive(Template)]
#[template(path = "clip.html")]
pub struct ClipPage {
    pub title: String,
    pub site_title: String,
    pub asset_v: &'static str,
    pub rail: &'static str,
    pub search_action: &'static str,
    pub q: String,
    pub media: Media,
    pub is_image: bool,
    pub sources: Vec<PlaybackSource>,
    /// Shown only when no source is playable: the original, as a last resort.
    pub fallback_source: Option<PlaybackSource>,
    /// A transcode is queued or running for this video.
    pub transcode_pending: bool,
    pub og_video: Option<PlaybackSource>,
    pub base: String,
    pub secure: bool,
    pub page_url: String,
    pub og_image: String,
    pub og_description: String,
    pub poster_url: String,
    pub original_url: String,
    pub download_url: String,
    pub start: String,
    pub meta_line: String,
    pub duration: String,
    pub resolution: String,
    pub size: String,
    pub bitrate: String,
    pub renditions: String,
    pub date_label: &'static str,
    pub date_full: String,
    pub views: i64,
    pub tags: Vec<TagView>,
    pub share_url: String,
    pub signed_in: bool,
    pub admin_csrf: Option<String>,
    pub is_scan: bool,
    pub tags_csv: String,
    pub quality: Vec<QualityRow>,
}

#[derive(Template)]
#[template(path = "image.html")]
pub struct ImagePage {
    pub title: String,
    pub site_title: String,
    pub asset_v: &'static str,
    pub rail: &'static str,
    pub search_action: &'static str,
    pub q: String,
    pub media: Media,
    pub is_image: bool,
    pub page_url: String,
    pub og_image: String,
    pub og_mime: String,
    pub og_description: String,
    pub original_url: String,
    pub download_url: String,
    pub meta_line: String,
    pub dimensions: String,
    pub format: String,
    pub file_name: String,
    pub size: String,
    pub date_label: &'static str,
    pub date_full: String,
    pub views: i64,
    pub tags: Vec<TagView>,
    pub share_url: String,
    pub signed_in: bool,
    pub admin_csrf: Option<String>,
    pub is_scan: bool,
    pub tags_csv: String,
    // Read by the shared Details panel; meaningless for images.
    pub duration: String,
    pub resolution: String,
    pub bitrate: String,
    pub renditions: String,
}

pub fn format_label(ext: &str) -> String {
    match ext.to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => "JPG".into(),
        e => e.to_ascii_uppercase(),
    }
}

fn parse_start(map: &HashMap<String, String>) -> String {
    match map.get("t").and_then(|t| t.parse::<f64>().ok()) {
        Some(t) if t.is_finite() && t > 0.0 && t < 86_400.0 * 7.0 => format!("{}", t.floor()),
        _ => "0".to_string(),
    }
}

/// `GET /w/{slug}`.
pub async fn video_page(
    state: State<AppState>,
    peer: ConnectInfo<SocketAddr>,
    admin: OptionalAdmin,
    headers: HeaderMap,
    slug: Path<String>,
    map: Query<HashMap<String, String>>,
) -> Result<Response, PageError> {
    clip_page(
        state.0,
        peer.0,
        headers,
        slug.0,
        map.0,
        MediaKind::Video,
        admin.0,
    )
    .await
}

/// `GET /i/{slug}`.
pub async fn image_page(
    state: State<AppState>,
    peer: ConnectInfo<SocketAddr>,
    admin: OptionalAdmin,
    headers: HeaderMap,
    slug: Path<String>,
    map: Query<HashMap<String, String>>,
) -> Result<Response, PageError> {
    clip_page(
        state.0,
        peer.0,
        headers,
        slug.0,
        map.0,
        MediaKind::Image,
        admin.0,
    )
    .await
}

async fn clip_page(
    state: AppState,
    peer: SocketAddr,
    headers: HeaderMap,
    slug: String,
    map: HashMap<String, String>,
    kind: MediaKind,
    admin: Option<crate::db::sessions::Session>,
) -> Result<Response, PageError> {
    let Some(m) = media::get_by_slug(&state.db, &slug).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    if m.kind != kind || !m.available {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let settings = state.settings.get();
    let trusted = &state.config.trusted_proxies;
    let ip = client_ip(&headers, peer.ip(), trusted);
    crate::db::views::record(
        &state.db,
        &m.id,
        &ip_hash(&state.settings.instance_secret(), ip),
    )
    .await?;
    let views = crate::db::views::count(&state.db, &m.id).await?;
    let base = public_base(&settings, &headers, peer.ip(), trusted);
    let secure = base.starts_with("https://") || request_is_https(&headers, peer.ip(), trusted);
    let is_image = kind == MediaKind::Image;
    if is_image {
        return image_response(&state, m, admin, base, views).await;
    }
    let renditions = jobs::renditions_for(&state.db, &m.id).await?;
    let sources = playback_sources(&m, &renditions);
    let og = og_video(&sources).cloned();
    let transcode_pending = renditions
        .iter()
        .any(|r| matches!(r.state.as_str(), "queued" | "running"));
    let fallback_source = if sources.is_empty() {
        let mime = mime_guess::from_ext(&m.extension)
            .first()
            .map(|x| x.essence_str().to_string())
            .unwrap_or_else(|| "video/mp4".into());
        Some(PlaybackSource {
            url: media_url(&m, &format!("original.{}", m.extension)),
            label: "Source".into(),
            mime,
        })
    } else {
        None
    };
    let quality = quality_rows(&settings.ladder, &renditions, admin.is_some());
    let tags = crate::db::tags::for_media(&state.db, &m.id).await?;

    let original_path = media_url(&m, &format!("original.{}", m.extension));
    let poster = if m.has_poster {
        media_url(&m, "poster.jpg")
    } else {
        PLACEHOLDER_URL.to_string()
    };
    let now = chrono::Utc::now();
    let resolution = fmt_resolution(&m);
    let local = when(&m).with_timezone(&chrono::Local);
    let mut meta_parts = vec![format!(
        "{} {}",
        if m.recorded_at.is_some() {
            "Recorded"
        } else {
            "Uploaded"
        },
        local.format("%a, %b %d %Y")
    )];
    if !resolution.is_empty() {
        meta_parts.push(resolution.clone());
    }
    meta_parts.push(fmt_size(m.size));
    meta_parts.push(fmt_views(views));
    let rendition_labels: Vec<String> = sources
        .iter()
        .filter(|s| s.label != "Source")
        .map(|s| s.label.clone())
        .collect();
    let og_description = if m.description.trim().is_empty() {
        format!("{} on {}", m.title, settings.site_title)
    } else {
        m.description.chars().take(300).collect()
    };
    let page = ClipPage {
        title: format!("{} - {}", m.title, settings.site_title),
        site_title: settings.site_title.clone(),
        asset_v: asset_v(),
        rail: "clips",
        search_action: "/",
        q: String::new(),
        is_image: false,
        og_video: og,
        secure,
        page_url: format!("{base}{}", page_path(&m)),
        og_image: format!("{base}{poster}"),
        og_description,
        poster_url: poster,
        download_url: format!("{original_path}?download=1"),
        original_url: original_path,
        start: parse_start(&map),
        meta_line: meta_parts.join(" · "),
        duration: fmt_duration(m.duration_ms),
        resolution,
        size: fmt_size(m.size),
        bitrate: m
            .bitrate
            .filter(|b| *b > 0)
            .map(|b| format!("{:.1} Mb/s", b as f64 / 1_000_000.0))
            .unwrap_or_default(),
        renditions: rendition_labels.join(", "),
        date_label: if m.recorded_at.is_some() {
            "Recorded"
        } else {
            "Uploaded"
        },
        date_full: format!(
            "{} ({})",
            local.format("%Y-%m-%d %H:%M"),
            fmt_relative(when(&m), now)
        ),
        views,
        tags: tags.iter().map(|t| TagView::new(t, "")).collect(),
        share_url: format!("{base}{}", page_path(&m)),
        signed_in: admin.is_some(),
        admin_csrf: admin.map(|s| s.csrf),
        is_scan: m.source == crate::media::kind::Source::Scan,
        tags_csv: tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        quality,
        sources,
        fallback_source,
        transcode_pending,
        base,
        media: m,
    };
    let mut resp = render(&page)?.into_response();
    if !page.signed_in {
        resp.extensions_mut().insert(AllowFraming);
    }
    Ok(resp)
}

async fn image_response(
    state: &AppState,
    m: Media,
    admin: Option<crate::db::sessions::Session>,
    base: String,
    views: i64,
) -> Result<Response, PageError> {
    let settings = state.settings.get();
    let tags = crate::db::tags::for_media(&state.db, &m.id).await?;
    let original_path = media_url(&m, &format!("original.{}", m.extension));
    let now = chrono::Utc::now();
    let local = when(&m).with_timezone(&chrono::Local);
    let date_label = if m.recorded_at.is_some() {
        "Recorded"
    } else {
        "Uploaded"
    };
    let dimensions = match (m.width, m.height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => format!("{w}\u{d7}{h}"),
        _ => String::new(),
    };
    let mut meta_parts = vec![format!("{date_label} {}", local.format("%a, %b %d %Y"))];
    if !dimensions.is_empty() {
        meta_parts.push(dimensions.clone());
    }
    meta_parts.push(fmt_size(m.size));
    meta_parts.push(fmt_views(views));
    let og_description = if m.description.trim().is_empty() {
        format!("{} on {}", m.title, settings.site_title)
    } else {
        m.description.chars().take(300).collect()
    };
    let og_mime = mime_guess::from_ext(&m.extension)
        .first()
        .map(|x| x.essence_str().to_string())
        .unwrap_or_else(|| "image/png".into());
    let page = ImagePage {
        title: format!("{} - {}", m.title, settings.site_title),
        site_title: settings.site_title.clone(),
        asset_v: asset_v(),
        rail: "images",
        search_action: "/images",
        q: String::new(),
        is_image: true,
        page_url: format!("{base}{}", page_path(&m)),
        og_image: format!("{base}{original_path}"),
        og_mime,
        og_description,
        download_url: format!("{original_path}?download=1"),
        original_url: original_path,
        meta_line: meta_parts.join(" \u{b7} "),
        dimensions,
        format: format_label(&m.extension),
        file_name: std::path::Path::new(&m.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: fmt_size(m.size),
        date_label,
        date_full: format!(
            "{} ({})",
            local.format("%Y-%m-%d %H:%M"),
            fmt_relative(when(&m), now)
        ),
        views,
        tags: tags.iter().map(|t| TagView::new(t, "")).collect(),
        share_url: format!("{base}{}", page_path(&m)),
        signed_in: admin.is_some(),
        admin_csrf: admin.map(|s| s.csrf),
        is_scan: m.source == crate::media::kind::Source::Scan,
        tags_csv: tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        duration: String::new(),
        resolution: String::new(),
        bitrate: String::new(),
        renditions: String::new(),
        media: m,
    };
    let mut resp = render(&page)?.into_response();
    if !page.signed_in {
        resp.extensions_mut().insert(AllowFraming);
    }
    Ok(resp)
}
