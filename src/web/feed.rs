//! `GET /feed.rss`: the newest public items as an RSS 2.0 feed.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use rss::extension::{Extension, ExtensionBuilder};
use rss::{ChannelBuilder, EnclosureBuilder, GuidBuilder, ItemBuilder};

use crate::app::AppState;
use crate::db::jobs;
use crate::db::media::{self, ListQuery, Media, Sort};
use crate::media::kind::MediaKind;
use crate::settings::public_base;
use crate::web::admin::OptionalAdmin;
use crate::web::clip::playback_sources;
use crate::web::library::{library_gate, media_url, page_path, poster_path, PageError};

const FEED_ITEMS: u32 = 50;

/// `(url path, length, mime)` of the best playable file, if any.
async fn enclosure_for(
    state: &AppState,
    m: &Media,
) -> anyhow::Result<Option<(String, i64, String)>> {
    if m.kind == MediaKind::Image {
        let mime = mime_guess::from_ext(&m.extension)
            .first()
            .map_or_else(|| "application/octet-stream".into(), |x| x.to_string());
        return Ok(Some((
            media_url(m, &format!("original.{}", m.extension)),
            m.size,
            mime,
        )));
    }
    let renditions = jobs::renditions_for(&state.db, &m.id).await?;
    let Some(best) = playback_sources(m, &renditions).into_iter().next() else {
        return Ok(None);
    };
    let length = if best.label == "Source" {
        m.size
    } else {
        renditions
            .iter()
            .find(|r| format!("{}p", r.height) == best.label)
            .map_or(0, |r| r.size)
    };
    Ok(Some((best.url, length, best.mime)))
}

pub async fn feed(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    admin: OptionalAdmin,
    headers: HeaderMap,
) -> Result<Response, PageError> {
    let settings = state.settings.get();
    if let Some(r) = library_gate(&settings, &admin, false, "/feed.rss") {
        return Ok(r);
    }
    let base = public_base(
        &settings,
        &headers,
        peer.ip(),
        &state.config.trusted_proxies,
    );
    let rows = media::list(
        &state.db,
        &ListQuery {
            kind: None,
            public_only: true,
            q: None,
            tag: None,
            sort: Sort::Newest,
            page: 1,
            per_page: FEED_ITEMS,
        },
    )
    .await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        let m = &row.media;
        let link = format!("{base}{}", page_path(m));
        let description = if m.description.trim().is_empty() {
            m.title.clone()
        } else {
            m.description.clone()
        };
        let mut b = ItemBuilder::default();
        b.title(m.title.clone())
            .link(link.clone())
            .guid(GuidBuilder::default().value(link).permalink(true).build())
            .pub_date(m.recorded_at.unwrap_or(m.created_at).to_rfc2822())
            .description(description);
        if let Some((url, length, mime)) = enclosure_for(&state, m).await? {
            b.enclosure(
                EnclosureBuilder::default()
                    .url(format!("{base}{url}"))
                    .length(length.to_string())
                    .mime_type(mime)
                    .build(),
            );
        }
        let has_thumb = match m.kind {
            MediaKind::Video => m.has_poster,
            MediaKind::Image => m.has_thumb,
        };
        if has_thumb {
            let ext = ExtensionBuilder::default()
                .name("media:thumbnail")
                .attrs(BTreeMap::from([(
                    "url".to_string(),
                    format!("{base}{}", poster_path(m)),
                )]))
                .build();
            b.extensions(BTreeMap::from([(
                "media".to_string(),
                BTreeMap::from([("thumbnail".to_string(), vec![ext])]),
            )]));
        }
        items.push(b.build());
    }

    let self_link: Extension = ExtensionBuilder::default()
        .name("atom:link")
        .attrs(BTreeMap::from([
            ("href".to_string(), format!("{base}/feed.rss")),
            ("rel".to_string(), "self".to_string()),
            ("type".to_string(), "application/rss+xml".to_string()),
        ]))
        .build();
    let channel = ChannelBuilder::default()
        .title(settings.site_title.clone())
        .link(base.clone())
        .description("Public clips")
        .language("en".to_string())
        .last_build_date(chrono::Utc::now().to_rfc2822())
        .namespaces(BTreeMap::from([
            (
                "media".to_string(),
                "http://search.yahoo.com/mrss/".to_string(),
            ),
            (
                "atom".to_string(),
                "http://www.w3.org/2005/Atom".to_string(),
            ),
        ]))
        .extensions(BTreeMap::from([(
            "atom".to_string(),
            BTreeMap::from([("link".to_string(), vec![self_link])]),
        )]))
        .items(items)
        .build();
    Ok((
        [
            (CONTENT_TYPE, "application/rss+xml; charset=utf-8"),
            (CACHE_CONTROL, "public, max-age=300"),
        ],
        channel.to_string(),
    )
        .into_response())
}
