//! Server-rendered admin pages: Files and Tags.

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::header::LOCATION;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;

use super::AdminSession;
use crate::app::AppState;
use crate::db::media::{self, ListQuery, MediaWithViews};
use crate::db::tags::{self, TagWithCount};
use crate::media::kind::{MediaKind, Source};
use crate::web::assets::asset_v;
use crate::web::library::{
    fmt_relative, fmt_size, page_path, poster_path, render, PageError, TagView,
};

const PER_PAGE: u32 = 50;

pub const TABS: [(&str, &str, &str); 7] = [
    ("files", "Files", "/admin/files"),
    ("tags", "Tags", "/admin/tags"),
    ("library", "Library", "/admin/library"),
    ("transcoding", "Transcoding", "/admin/transcoding"),
    ("sharing", "Sharing", "/admin/sharing"),
    ("integrations", "Integrations", "/admin/integrations"),
    ("tokens", "Tokens", "/admin/tokens"),
];

/// `GET /admin`.
pub async fn root(AdminSession(_): AdminSession) -> Response {
    (StatusCode::SEE_OTHER, [(LOCATION, "/admin/files")]).into_response()
}

pub struct FileRow {
    pub id: String,
    pub slug: String,
    pub source: &'static str,
    pub href: String,
    pub poster_url: String,
    pub title: String,
    pub public: bool,
    pub tags: Vec<TagView>,
    pub views: i64,
    pub size: String,
    pub status: &'static str,
    pub status_class: &'static str,
    pub error: String,
    pub added: String,
}

fn file_row(r: &MediaWithViews, now: chrono::DateTime<Utc>) -> FileRow {
    let m = &r.media;
    let (status, status_class) = match m.processing_state.as_str() {
        "ready" => ("ready", "pub"),
        "failed" => ("failed", "err"),
        _ => ("pending", "proc"),
    };
    FileRow {
        id: m.id.clone(),
        slug: m.slug.clone(),
        source: match m.source {
            Source::Scan => "scan",
            Source::Upload => "upload",
        },
        href: page_path(m),
        poster_url: poster_path(m),
        title: m.title.clone(),
        public: m.public,
        tags: r.tags.iter().map(|t| TagView::new(t, "")).collect(),
        views: r.views,
        size: fmt_size(m.size),
        status,
        status_class,
        error: m.processing_error.clone().unwrap_or_default(),
        added: fmt_relative(m.created_at, now),
    }
}

#[derive(Template)]
#[template(path = "admin/files.html")]
pub struct FilesPage {
    pub title: String,
    pub site_title: String,
    pub asset_v: &'static str,
    pub rail: &'static str,
    pub search_action: &'static str,
    pub q: String,
    pub signed_in: bool,
    pub admin_csrf: Option<String>,
    pub active: &'static str,
    pub tabs: [(&'static str, &'static str, &'static str); 7],
    pub rows: Vec<FileRow>,
    pub kind: String,
    pub total: i64,
    pub count_clips: i64,
    pub count_images: i64,
    pub page: u32,
    pub pages: u32,
    pub prev_href: String,
    pub next_href: String,
    pub all_href: String,
    pub clips_href: String,
    pub images_href: String,
}

fn href(q: &str, kind: &str, page: u32) -> String {
    let mut parts = Vec::new();
    if !q.is_empty() {
        parts.push(format!("q={}", urlencoding::encode(q)));
    }
    if !kind.is_empty() {
        parts.push(format!("kind={kind}"));
    }
    if page > 1 {
        parts.push(format!("page={page}"));
    }
    if parts.is_empty() {
        "/admin/files".into()
    } else {
        format!("/admin/files?{}", parts.join("&"))
    }
}

/// `GET /admin/files?page=&q=&kind=`.
pub async fn files(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
    Query(map): Query<HashMap<String, String>>,
) -> Result<Response, PageError> {
    let q = map
        .get("q")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let (kind, kind_filter) = match map.get("kind").map(String::as_str) {
        Some("video") => ("video", Some(MediaKind::Video)),
        Some("image") => ("image", Some(MediaKind::Image)),
        _ => ("", None),
    };
    let page = map
        .get("page")
        .and_then(|p| p.parse::<u32>().ok())
        .filter(|p| *p >= 1)
        .unwrap_or(1);
    let query = ListQuery {
        kind: kind_filter,
        public_only: false,
        q: Some(q.clone()).filter(|s| !s.is_empty()),
        tag: None,
        sort: state.settings.get().default_sort,
        page,
        per_page: PER_PAGE,
    };
    let total = media::count_matching(&state.db, &query).await?;
    let list = media::list(&state.db, &query).await?;
    let now = Utc::now();
    let pages = u32::try_from((total + i64::from(PER_PAGE) - 1) / i64::from(PER_PAGE))
        .unwrap_or(1)
        .max(1);
    let site_title = state.settings.get().site_title.clone();
    let page_view = FilesPage {
        title: format!("Files - {site_title}"),
        site_title,
        asset_v: asset_v(),
        rail: "admin",
        search_action: "/admin/files",
        signed_in: true,
        admin_csrf: Some(session.csrf),
        active: "files",
        tabs: TABS,
        rows: list.iter().map(|r| file_row(r, now)).collect(),
        total,
        count_clips: media::count(&state.db, Some(MediaKind::Video), false).await?,
        count_images: media::count(&state.db, Some(MediaKind::Image), false).await?,
        page,
        pages,
        prev_href: href(&q, kind, page.saturating_sub(1).max(1)),
        next_href: href(&q, kind, page.saturating_add(1)),
        all_href: href(&q, "", 1),
        clips_href: href(&q, "video", 1),
        images_href: href(&q, "image", 1),
        kind: kind.to_string(),
        q,
    };
    Ok(render(&page_view)?.into_response())
}

#[derive(Template)]
#[template(path = "admin/tags.html")]
pub struct TagsPage {
    pub title: String,
    pub site_title: String,
    pub asset_v: &'static str,
    pub rail: &'static str,
    pub search_action: &'static str,
    pub q: String,
    pub signed_in: bool,
    pub admin_csrf: Option<String>,
    pub active: &'static str,
    pub tabs: [(&'static str, &'static str, &'static str); 7],
    pub tags: Vec<TagWithCount>,
    pub default_colour: &'static str,
}

/// `GET /admin/tags`.
pub async fn tags(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
) -> Result<Response, PageError> {
    let site_title = state.settings.get().site_title.clone();
    let page = TagsPage {
        title: format!("Tags - {site_title}"),
        site_title,
        asset_v: asset_v(),
        rail: "admin",
        search_action: "/admin/files",
        q: String::new(),
        signed_in: true,
        admin_csrf: Some(session.csrf),
        active: "tags",
        tabs: TABS,
        tags: tags::list_with_counts(&state.db).await?,
        default_colour: tags::DEFAULT_COLOUR,
    };
    Ok(render(&page)?.into_response())
}
