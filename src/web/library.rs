//! Library pages (`/`, `/images`) and the JSON API (`/api/library`). Both go
//! through [`run_query`] so they can never disagree about what is listed.

use std::collections::HashMap;
use std::net::SocketAddr;

use askama::Template;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::header::LOCATION;
use axum::http::Uri;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Local, Utc};
use serde::Serialize;

use crate::app::AppState;
use crate::db::media::{self, ListQuery, Media, MediaWithViews, Sort};
use crate::db::tags::{self, Tag};
use crate::media::kind::MediaKind;
use crate::settings::public_base;
use crate::web::admin::OptionalAdmin;
use crate::web::assets::asset_v;

pub const PER_PAGE: u32 = 48;
pub const PLACEHOLDER_URL: &str = "/static/poster-placeholder.svg";

/// Any handler failure: logged, shown as a bare 500.
pub struct PageError(pub anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for PageError {
    fn from(e: E) -> Self {
        PageError(e.into())
    }
}

impl IntoResponse for PageError {
    fn into_response(self) -> Response {
        tracing::error!(error = %self.0, "page error");
        (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
    }
}

pub fn render<T: Template>(t: &T) -> Result<Html<String>, PageError> {
    Ok(Html(t.render().map_err(anyhow::Error::from)?))
}

#[derive(Debug, Clone)]
pub struct Params {
    pub kind: MediaKind,
    pub q: String,
    pub tag: String,
    pub sort: Sort,
    pub page: u32,
}

pub fn parse_sort(s: &str) -> Option<Sort> {
    Some(match s {
        "newest" => Sort::Newest,
        "oldest" => Sort::Oldest,
        "views" => Sort::MostViewed,
        "title" => Sort::Title,
        "longest" => Sort::Longest,
        _ => return None,
    })
}

fn sort_name(s: Sort) -> &'static str {
    match s {
        Sort::Newest => "newest",
        Sort::Oldest => "oldest",
        Sort::MostViewed => "views",
        Sort::Title => "title",
        Sort::Longest => "longest",
    }
}

impl Params {
    pub fn from_query(
        map: &HashMap<String, String>,
        kind: Option<MediaKind>,
        default_sort: Sort,
    ) -> Params {
        let kind = kind.unwrap_or(match map.get("kind").map(String::as_str) {
            Some("image") => MediaKind::Image,
            _ => MediaKind::Video,
        });
        Params {
            kind,
            q: map
                .get("q")
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            tag: map
                .get("tag")
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            sort: map
                .get("sort")
                .and_then(|s| parse_sort(s))
                .unwrap_or(default_sort),
            page: map
                .get("page")
                .and_then(|p| p.parse::<u32>().ok())
                .filter(|p| *p >= 1)
                .unwrap_or(1),
        }
    }
}

pub struct Listing {
    pub rows: Vec<MediaWithViews>,
    pub total: i64,
}

/// The one listing query: public and available rows only.
pub async fn run_query(state: &AppState, p: &Params) -> anyhow::Result<Listing> {
    let q = ListQuery {
        kind: Some(p.kind),
        public_only: true,
        q: Some(p.q.clone()).filter(|s| !s.is_empty()),
        tag: Some(p.tag.clone()).filter(|s| !s.is_empty()),
        sort: p.sort,
        page: p.page,
        per_page: PER_PAGE,
    };
    let total = media::count_matching(&state.db, &q).await?;
    let rows = media::list(&state.db, &q).await?;
    Ok(Listing { rows, total })
}

// ---- formatting helpers shared with clip.rs ----

pub fn fmt_duration(ms: Option<i64>) -> String {
    let Some(ms) = ms.filter(|m| *m >= 0) else {
        return String::new();
    };
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn fmt_size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    const KB: f64 = 1024.0;
    if b < KB {
        format!("{} B", bytes.max(0))
    } else if b < KB * KB {
        format!("{:.0} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.0} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

pub fn fmt_resolution(m: &Media) -> String {
    match m.height {
        Some(h) if h > 0 => match m.fps {
            Some(f) if f >= 50.0 => format!("{h}p{}", f.round() as i64),
            _ => format!("{h}p"),
        },
        _ => String::new(),
    }
}

pub fn when(m: &Media) -> DateTime<Utc> {
    m.recorded_at.unwrap_or(m.created_at)
}

pub fn fmt_relative(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - then).num_seconds();
    if secs < 60 {
        return "just now".into();
    }
    if secs < 3600 {
        return format!("{}m ago", secs / 60);
    }
    if secs < 86_400 {
        return format!("{}h ago", secs / 3600);
    }
    let day_of = |d: DateTime<Utc>| d.with_timezone(&Local).date_naive();
    if (day_of(now) - day_of(then)).num_days() == 1 {
        return "yesterday".into();
    }
    day_of(then).format("%b %-d, %Y").to_string()
}

pub fn fmt_views(n: i64) -> String {
    if n == 1 {
        "1 view".into()
    } else {
        format!("{n} views")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TagView {
    pub name: String,
    pub colour: String,
    #[serde(skip)]
    pub name_enc: String,
    #[serde(skip)]
    pub on: bool,
}

impl TagView {
    pub fn new(t: &Tag, active: &str) -> TagView {
        TagView {
            name: t.name.clone(),
            colour: t.colour.clone(),
            name_enc: urlencoding::encode(&t.name).into_owned(),
            on: !active.is_empty() && t.name == active,
        }
    }
}

pub fn media_url(m: &Media, file: &str) -> String {
    format!("/media/{}/{file}", m.slug)
}

pub fn page_path(m: &Media) -> String {
    let prefix = match m.kind {
        MediaKind::Video => "w",
        MediaKind::Image => "i",
    };
    format!("/{prefix}/{}", m.slug)
}

pub fn poster_path(m: &Media) -> String {
    match m.kind {
        MediaKind::Video if m.has_poster => media_url(m, "poster.jpg"),
        MediaKind::Image if m.has_thumb => media_url(m, "thumb.jpg"),
        _ => PLACEHOLDER_URL.to_string(),
    }
}

pub struct CardView {
    pub href: String,
    pub title: String,
    pub poster_url: String,
    pub preview_url: Option<String>,
    pub res: String,
    pub duration: String,
    pub is_image: bool,
    pub meta: String,
    pub share_url: String,
    pub tags: Vec<TagView>,
}

pub struct DateSection {
    pub label: String,
    pub items: Vec<CardView>,
}

pub struct SortOption {
    pub value: &'static str,
    pub label: &'static str,
    pub selected: bool,
}

#[derive(Template)]
#[template(path = "library.html")]
pub struct LibraryPage {
    pub title: String,
    pub heading: String,
    pub site_title: String,
    pub asset_v: &'static str,
    pub rail: &'static str,
    pub search_action: &'static str,
    pub kind_video: bool,
    pub tag_path: &'static str,
    pub sections: Vec<DateSection>,
    pub count_clips: i64,
    pub count_images: i64,
    pub tags: Vec<TagView>,
    pub q: String,
    pub tag: String,
    pub keep_q: String,
    pub sort_options: Vec<SortOption>,
    pub page: u32,
    pub pages: u32,
    pub prev_href: String,
    pub next_href: String,
    pub signed_in: bool,
    pub admin_csrf: Option<String>,
}

fn card(item: &MediaWithViews, base: &str, now: DateTime<Utc>) -> CardView {
    let m = &item.media;
    let is_image = m.kind == MediaKind::Image;
    CardView {
        href: page_path(m),
        title: m.title.clone(),
        poster_url: poster_path(m),
        preview_url: (m.has_preview && !is_image).then(|| media_url(m, "preview.mp4")),
        res: if is_image {
            crate::web::clip::format_label(&m.extension)
        } else {
            fmt_resolution(m)
        },
        duration: fmt_duration(m.duration_ms),
        is_image,
        meta: format!(
            "{} · {} · {}",
            fmt_size(m.size),
            fmt_relative(when(m), now),
            fmt_views(item.views)
        ),
        share_url: format!("{base}{}", page_path(m)),
        tags: item.tags.iter().map(|t| TagView::new(t, "")).collect(),
    }
}

fn sections(rows: &[MediaWithViews], by_date: bool, base: &str) -> Vec<DateSection> {
    let now = Utc::now();
    let mut out: Vec<DateSection> = Vec::new();
    let mut last: Option<chrono::NaiveDate> = None;
    for r in rows {
        let day = when(&r.media).with_timezone(&Local).date_naive();
        let start_new = match (by_date, last) {
            (_, None) => true,
            (true, Some(d)) => d != day,
            (false, Some(_)) => false,
        };
        if start_new {
            out.push(DateSection {
                label: if by_date {
                    day.format("%a, %b %d").to_string()
                } else {
                    String::new()
                },
                items: Vec::new(),
            });
        }
        last = Some(day);
        if let Some(s) = out.last_mut() {
            s.items.push(card(r, base, now));
        }
    }
    out
}

fn href_with(path: &str, p: &Params, page: u32) -> String {
    let mut parts = Vec::new();
    if !p.q.is_empty() {
        parts.push(format!("q={}", urlencoding::encode(&p.q)));
    }
    if !p.tag.is_empty() {
        parts.push(format!("tag={}", urlencoding::encode(&p.tag)));
    }
    parts.push(format!("sort={}", sort_name(p.sort)));
    parts.push(format!("page={page}"));
    format!("{path}?{}", parts.join("&"))
}

async fn library_page(
    state: AppState,
    peer: SocketAddr,
    headers: HeaderMap,
    map: HashMap<String, String>,
    kind: MediaKind,
    admin: Option<crate::db::sessions::Session>,
) -> Result<Html<String>, PageError> {
    let settings = state.settings.get();
    let p = Params::from_query(&map, Some(kind), settings.default_sort);
    let listing = run_query(&state, &p).await?;
    let base = public_base(
        &settings,
        &headers,
        peer.ip(),
        &state.config.trusted_proxies,
    );
    let is_video = kind == MediaKind::Video;
    let path = if is_video { "/" } else { "/images" };
    let by_date = matches!(p.sort, Sort::Newest | Sort::Oldest);
    let all_tags = tags::list_for_public(&state.db, Some(kind)).await?;
    let pages = u32::try_from((listing.total.max(0) as u64).div_ceil(u64::from(PER_PAGE)))
        .unwrap_or(u32::MAX);
    let sort_options = [
        ("newest", "Newest first", Sort::Newest),
        ("oldest", "Oldest first", Sort::Oldest),
        ("views", "Most viewed", Sort::MostViewed),
        ("title", "Title", Sort::Title),
        ("longest", "Longest", Sort::Longest),
    ]
    .into_iter()
    .map(|(value, label, s)| SortOption {
        value,
        label,
        selected: s == p.sort,
    })
    .collect();
    let page = LibraryPage {
        title: settings.site_title.clone(),
        heading: if is_video {
            "Public library".into()
        } else {
            "Screenshots".into()
        },
        site_title: settings.site_title.clone(),
        asset_v: asset_v(),
        rail: if is_video { "clips" } else { "images" },
        search_action: path,
        kind_video: is_video,
        tag_path: path,
        sections: sections(&listing.rows, by_date, &base),
        count_clips: media::count(&state.db, Some(MediaKind::Video), true).await?,
        count_images: media::count(&state.db, Some(MediaKind::Image), true).await?,
        tags: all_tags.iter().map(|t| TagView::new(t, &p.tag)).collect(),
        keep_q: if p.q.is_empty() {
            String::new()
        } else {
            format!("?q={}", urlencoding::encode(&p.q))
        },
        q: p.q.clone(),
        tag: p.tag.clone(),
        sort_options,
        page: p.page,
        pages,
        prev_href: href_with(path, &p, p.page.saturating_sub(1).max(1)),
        next_href: href_with(path, &p, p.page.saturating_add(1)),
        signed_in: admin.is_some(),
        admin_csrf: admin.map(|s| s.csrf),
    };
    render(&page)
}

/// Enforces `library_requires_login`: `None` lets the request through,
/// otherwise the response to send instead. Pages (`wants_html`) redirect to
/// the login form and come back to `next`; APIs and feeds answer 401 JSON.
pub fn library_gate(
    settings: &crate::settings::Settings,
    admin: &OptionalAdmin,
    wants_html: bool,
    next: &str,
) -> Option<Response> {
    if !settings.library_requires_login || admin.0.is_some() {
        return None;
    }
    if wants_html {
        let next = crate::web::admin::safe_next(Some(next));
        let location = format!("/admin/login?next={}", urlencoding::encode(next));
        Some((StatusCode::SEE_OTHER, [(LOCATION, location)]).into_response())
    } else {
        Some(
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
                .into_response(),
        )
    }
}

/// `GET /`.
pub async fn videos(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    admin: OptionalAdmin,
    uri: Uri,
    headers: HeaderMap,
    Query(map): Query<HashMap<String, String>>,
) -> Result<Response, PageError> {
    let next = uri.path_and_query().map_or("/", |p| p.as_str());
    if let Some(r) = library_gate(&state.settings.get(), &admin, true, next) {
        return Ok(r);
    }
    Ok(
        library_page(state, peer, headers, map, MediaKind::Video, admin.0)
            .await?
            .into_response(),
    )
}

/// `GET /images`.
pub async fn images(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    admin: OptionalAdmin,
    uri: Uri,
    headers: HeaderMap,
    Query(map): Query<HashMap<String, String>>,
) -> Result<Response, PageError> {
    let next = uri.path_and_query().map_or("/", |p| p.as_str());
    if let Some(r) = library_gate(&state.settings.get(), &admin, true, next) {
        return Ok(r);
    }
    Ok(
        library_page(state, peer, headers, map, MediaKind::Image, admin.0)
            .await?
            .into_response(),
    )
}

#[derive(Serialize)]
pub struct LibraryItem {
    pub id: String,
    pub slug: String,
    pub kind: &'static str,
    pub title: String,
    pub url: String,
    pub poster_url: String,
    pub preview_url: Option<String>,
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub size: i64,
    pub views: i64,
    pub created_at: DateTime<Utc>,
    pub recorded_at: Option<DateTime<Utc>>,
    pub tags: Vec<TagView>,
}

impl LibraryItem {
    /// Absolute URLs are `base` + path (`base` has no trailing slash).
    pub fn build(r: &MediaWithViews, base: &str) -> LibraryItem {
        let m = &r.media;
        LibraryItem {
            id: m.id.clone(),
            slug: m.slug.clone(),
            kind: m.kind.as_str(),
            title: m.title.clone(),
            url: format!("{base}{}", page_path(m)),
            poster_url: format!("{base}{}", poster_path(m)),
            preview_url: (m.has_preview && m.kind == MediaKind::Video)
                .then(|| format!("{base}{}", media_url(m, "preview.mp4"))),
            duration_ms: m.duration_ms,
            width: m.width,
            height: m.height,
            size: m.size,
            views: r.views,
            created_at: m.created_at,
            recorded_at: m.recorded_at,
            tags: r.tags.iter().map(|t| TagView::new(t, "")).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct LibraryResponse {
    pub items: Vec<LibraryItem>,
    pub page: u32,
    pub total: i64,
}

/// `GET /api/library?kind=&q=&tag=&sort=&page=`.
pub async fn api_library(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    admin: OptionalAdmin,
    headers: HeaderMap,
    Query(map): Query<HashMap<String, String>>,
) -> Result<Response, PageError> {
    let settings = state.settings.get();
    if let Some(r) = library_gate(&settings, &admin, false, "/") {
        return Ok(r);
    }
    let p = Params::from_query(&map, None, settings.default_sort);
    let listing = run_query(&state, &p).await?;
    let base = public_base(
        &settings,
        &headers,
        peer.ip(),
        &state.config.trusted_proxies,
    );
    let items = listing
        .rows
        .iter()
        .map(|r| LibraryItem::build(r, &base))
        .collect();
    Ok(Json(LibraryResponse {
        items,
        page: p.page,
        total: listing.total,
    })
    .into_response())
}
