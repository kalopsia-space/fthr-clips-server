//! Server-rendered admin pages: Files and Tags.

use std::collections::HashMap;

use askama::Template;
use axum::extract::{Query, State};
use axum::http::header::LOCATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Form;
use chrono::Utc;

use super::api_settings::{form_to_patch, validate};
use super::{require_csrf, AdminSession};
use crate::app::AppState;
use crate::db::media::{self, ListQuery, MediaWithViews};
use crate::db::sessions::Session;
use crate::db::tags::{self, TagWithCount};
use crate::media::kind::{MediaKind, Source};
use crate::settings::Settings;
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

pub struct TokenRow {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub created: String,
    pub last_used: String,
    pub revoked: bool,
}

#[derive(Template)]
#[template(path = "admin/tokens.html")]
pub struct TokensPage {
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
    pub tokens: Vec<TokenRow>,
}

/// `GET /admin/tokens`.
pub async fn tokens(
    AdminSession(session): AdminSession,
    State(state): State<AppState>,
) -> Result<Response, PageError> {
    let site_title = state.settings.get().site_title.clone();
    let day = |d: chrono::DateTime<chrono::Utc>| d.format("%Y-%m-%d %H:%M").to_string();
    let rows = crate::db::tokens::list(&state.db)
        .await?
        .into_iter()
        .rev()
        .map(|t| TokenRow {
            id: t.id,
            name: t.name,
            prefix: t.prefix,
            created: day(t.created_at),
            last_used: t.last_used_at.map(day).unwrap_or_else(|| "never".into()),
            revoked: t.revoked_at.is_some(),
        })
        .collect();
    let page = TokensPage {
        title: format!("Tokens - {site_title}"),
        site_title,
        asset_v: asset_v(),
        rail: "admin",
        search_action: "/admin/files",
        q: String::new(),
        signed_in: true,
        admin_csrf: Some(session.csrf),
        active: "tokens",
        tabs: TABS,
        tokens: rows,
    };
    Ok(render(&page)?.into_response())
}

/// Plain-string view of the settings for the form templates.
pub struct SettingsView {
    pub site_title: String,
    pub public_base_url: String,
    pub library_requires_login: bool,
    pub default_sort: String,
    pub upload_default_public: bool,
    pub upload_folder: String,
    pub scan_interval_minutes: u64,
    pub video_exts: String,
    pub image_exts: String,
    pub transcoding: String,
    pub encoder: String,
    pub ladder_480: bool,
    pub ladder_720: bool,
    pub ladder_1080: bool,
    pub transcode_workers: usize,
    pub transcode_timeout_minutes: u64,
    pub discord_webhook_url: String,
    pub webhook_url: String,
    pub webhooks_enabled: bool,
}

fn enum_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl SettingsView {
    fn new(s: &Settings) -> Self {
        Self {
            site_title: s.site_title.clone(),
            public_base_url: s.public_base_url.clone().unwrap_or_default(),
            library_requires_login: s.library_requires_login,
            default_sort: enum_name(&s.default_sort),
            upload_default_public: s.upload_default_public,
            upload_folder: s.upload_folder.clone(),
            scan_interval_minutes: s.scan_interval_minutes,
            video_exts: s.video_exts.join(", "),
            image_exts: s.image_exts.join(", "),
            transcoding: enum_name(&s.transcoding),
            encoder: enum_name(&s.encoder),
            ladder_480: s.ladder.contains(&480),
            ladder_720: s.ladder.contains(&720),
            ladder_1080: s.ladder.contains(&1080),
            transcode_workers: s.transcode_workers,
            transcode_timeout_minutes: s.transcode_timeout_minutes,
            discord_webhook_url: s.discord_webhook_url.clone().unwrap_or_default(),
            webhook_url: s.webhook_url.clone().unwrap_or_default(),
            webhooks_enabled: s.webhooks_enabled,
        }
    }
}

macro_rules! settings_page {
    ($name:ident, $path:literal) => {
        #[derive(Template)]
        #[template(path = $path)]
        pub struct $name {
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
            pub saved: bool,
            pub error: String,
            pub s: SettingsView,
            pub video_dirs: Vec<String>,
            pub image_dirs: Vec<String>,
            pub effective_encoder: String,
        }
    };
}

settings_page!(LibraryPage, "admin/library.html");
settings_page!(TranscodingPage, "admin/transcoding.html");
settings_page!(SharingPage, "admin/sharing.html");
settings_page!(IntegrationsPage, "admin/integrations.html");

async fn settings_page_response(
    tab: &'static str,
    label: &str,
    session: Session,
    state: &AppState,
    map: &HashMap<String, String>,
) -> Result<Response, PageError> {
    let settings = state.settings.get();
    let site_title = settings.site_title.clone();
    macro_rules! build {
        ($ty:ident) => {
            render(&$ty {
                title: format!("{label} - {site_title}"),
                site_title: site_title.clone(),
                asset_v: asset_v(),
                rail: "admin",
                search_action: "/admin/files",
                q: String::new(),
                signed_in: true,
                admin_csrf: Some(session.csrf.clone()),
                active: tab,
                tabs: TABS,
                saved: map.get("saved").is_some_and(|v| v == "1"),
                error: map.get("error").cloned().unwrap_or_default(),
                s: SettingsView::new(&settings),
                video_dirs: dirs(&state.config.video_dirs),
                image_dirs: dirs(&state.config.image_dirs),
                effective_encoder: match super::api_settings::effective_encoder(state).await {
                    Some(name) => name.to_string(),
                    None if state.ffmpeg.available() => String::new(),
                    None => "ffmpeg not found".to_string(),
                },
            })?
            .into_response()
        };
    }
    Ok(match tab {
        "library" => build!(LibraryPage),
        "transcoding" => build!(TranscodingPage),
        "sharing" => build!(SharingPage),
        _ => build!(IntegrationsPage),
    })
}

fn dirs(v: &[std::path::PathBuf]) -> Vec<String> {
    v.iter().map(|p| p.display().to_string()).collect()
}

macro_rules! settings_handlers {
    ($get:ident, $post:ident, $tab:literal, $label:literal) => {
        /// `GET /admin/<tab>`.
        pub async fn $get(
            AdminSession(session): AdminSession,
            State(state): State<AppState>,
            Query(map): Query<HashMap<String, String>>,
        ) -> Result<Response, PageError> {
            settings_page_response($tab, $label, session, &state, &map).await
        }

        /// `POST /admin/<tab>`.
        pub async fn $post(
            AdminSession(session): AdminSession,
            State(state): State<AppState>,
            headers: HeaderMap,
            Form(form): Form<Vec<(String, String)>>,
        ) -> Response {
            save_settings($tab, &session, &state, &headers, &form).await
        }
    };
}

settings_handlers!(library_get, library_post, "library", "Library");
settings_handlers!(
    transcoding_get,
    transcoding_post,
    "transcoding",
    "Transcoding"
);
settings_handlers!(sharing_get, sharing_post, "sharing", "Sharing");
settings_handlers!(
    integrations_get,
    integrations_post,
    "integrations",
    "Integrations"
);

async fn save_settings(
    tab: &str,
    session: &Session,
    state: &AppState,
    headers: &HeaderMap,
    form: &[(String, String)],
) -> Response {
    let token = form
        .iter()
        .find(|(k, _)| k == "csrf")
        .map(|(_, v)| v.as_str());
    if let Err(code) = require_csrf(session, headers, token) {
        return code.into_response();
    }
    let old_encoder = state.settings.get().encoder;
    let result = match form_to_patch(tab, form) {
        Ok(mut patch) => match validate(&mut patch) {
            Ok(()) => state
                .settings
                .update(&state.db, patch)
                .await
                .map(|new| {
                    if new.encoder != old_encoder {
                        super::api_settings::redetect(state, new.encoder);
                    }
                })
                .map_err(|e| format!("{e:#}")),
            Err(e) => Err(e),
        },
        Err(e) => Err(e),
    };
    let location = match result {
        Ok(()) => format!("/admin/{tab}?saved=1"),
        Err(msg) => format!("/admin/{tab}?error={}", urlencoding::encode(&msg)),
    };
    (StatusCode::SEE_OTHER, [(LOCATION, location)]).into_response()
}
