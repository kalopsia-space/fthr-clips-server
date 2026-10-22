//! One-time announcement webhooks (Discord and generic JSON).
//!
//! Fired when a media item is public, available and has a poster/thumb.
//! The `webhook_sent` row is claimed atomically first, so the announcement
//! happens at most once per item. The HTTP sends are spawned and never
//! retried; if every send fails the claim is released so a later trigger
//! (re-publishing, another job finishing) can try again.
//! Known limitation: no SSRF guard; webhook URLs are admin-provided.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

use crate::app::AppState;
use crate::db::media::MediaWithViews;
use crate::db::{media, tags};
use crate::web::library::LibraryItem;

static WARNED_BASE: AtomicBool = AtomicBool::new(false);

fn base_url(state: &AppState) -> String {
    if let Some(b) = state
        .settings
        .get()
        .public_base_url
        .as_deref()
        .map(|b| b.trim_end_matches('/'))
        .filter(|b| !b.is_empty())
    {
        return b.to_string();
    }
    if !WARNED_BASE.swap(true, Ordering::Relaxed) {
        tracing::warn!("webhooks: PUBLIC_BASE_URL is not set; links will use the bind address");
    }
    format!("http://{}:{}", state.config.bind_host, state.config.port)
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "?".into())
}

fn discord_body(item: &LibraryItem, description: &str) -> Value {
    let desc: String = description.trim().chars().take(200).collect();
    json!({"embeds": [{
        "title": item.title,
        "url": item.url,
        "description": desc.trim(),
        "image": {"url": item.poster_url},
        "color": 2155424,
    }]})
}

async fn post(client: &reqwest::Client, url: &str, body: &Value) -> bool {
    match client.post(url).json(body).send().await {
        Ok(r) if r.status().is_success() => true,
        Ok(r) => {
            tracing::warn!(host = %host_of(url), status = %r.status(), "webhook rejected");
            false
        }
        Err(e) => {
            tracing::warn!(host = %host_of(url), error = %e.without_url(), "webhook failed");
            false
        }
    }
}

pub async fn maybe_announce(state: &AppState, media_id: &str) {
    let settings = state.settings.get();
    let discord = settings
        .discord_webhook_url
        .clone()
        .filter(|u| !u.is_empty());
    let generic = settings.webhook_url.clone().filter(|u| !u.is_empty());
    if !settings.webhooks_enabled || (discord.is_none() && generic.is_none()) {
        return;
    }
    if let Err(e) = announce(state, media_id, discord, generic).await {
        tracing::warn!(error = %e, "webhook announcement failed");
    }
}

async fn announce(
    state: &AppState,
    id: &str,
    discord: Option<String>,
    generic: Option<String>,
) -> anyhow::Result<()> {
    let Some(m) = media::get(&state.db, id).await? else {
        return Ok(());
    };
    if !(m.public && m.available && (m.has_poster || m.has_thumb)) {
        return Ok(());
    }
    if !media::claim_webhook(&state.db, id).await? {
        return Ok(());
    }
    let views = media::view_count(&state.db, id).await?;
    let row = MediaWithViews {
        tags: tags::for_media(&state.db, id).await?,
        media: m,
        views,
    };
    let item = LibraryItem::build(&row, &base_url(state));
    let discord_body = discord_body(&item, &row.media.description);
    let mut generic_body = serde_json::to_value(&item)?;
    // Only public items are announced.
    generic_body["public"] = Value::Bool(true);
    let (client, db, id) = (state.http.clone(), state.db.clone(), id.to_string());
    tokio::spawn(async move {
        let mut any_ok = false;
        let mut any_sent = false;
        if let Some(u) = discord {
            any_sent = true;
            any_ok |= post(&client, &u, &discord_body).await;
        }
        if let Some(u) = generic {
            any_sent = true;
            any_ok |= post(&client, &u, &generic_body).await;
        }
        if any_sent && !any_ok {
            if let Err(e) = media::release_webhook(&db, &id).await {
                tracing::warn!(error = %e, "could not release webhook claim");
            }
        }
    });
    Ok(())
}
