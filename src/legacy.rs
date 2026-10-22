//! Import of the old Python service's data dir: `{id}` + `{id}.json` pairs.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Deserialize;

use crate::app::AppState;
use crate::db::media::{self, NewMedia};
use crate::db::Source;
use crate::media::filename_date::recorded_at_from_name;
use crate::media::ids::{media_id, slug_candidates};
use crate::media::kind::kind_for;
use crate::web::upload::{basename, claim_path, safe_folder, sanitise_name};

const SNIFF_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: u32,
    pub skipped: u32,
}

#[derive(Deserialize)]
struct Sidecar {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default = "default_listed")]
    listed: bool,
}

fn default_listed() -> bool {
    true
}

/// `legacy_id.media_id` value for pairs that cannot be imported (unknown type,
/// unusable sidecar). They count as handled so they are not retried or re-warned
/// on every start; their files are left untouched and `/files/{old_id}` is 404.
const UNIMPORTABLE: &str = "";

/// Prefix of this module's temp links, so a crashed run's leftovers can be swept
/// without touching uploads in flight (which use `.incoming-` + hex).
const TEMP_PREFIX: &str = ".incoming-legacy-";

/// Old ids are 20..=32 chars of `[A-Za-z0-9_-]`.
pub fn valid_old_id(id: &str) -> bool {
    (20..=32).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Regular (non-symlink) files directly in `data_dir` that have a sibling sidecar.
fn find_pairs(data_dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(data_dir) else {
        return Vec::new();
    };
    let is_file = |p: &Path| {
        std::fs::symlink_metadata(p)
            .map(|m| m.file_type().is_file())
            .unwrap_or(false)
    };
    let mut ids: Vec<String> = rd
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_old_id(n))
        .filter(|n| is_file(&data_dir.join(n)) && is_file(&data_dir.join(format!("{n}.json"))))
        .collect();
    ids.sort();
    ids
}

/// Startup hook: imports when at least one pair is not yet in `legacy_id`.
pub async fn import_if_needed(state: &AppState) {
    let pending = {
        let mut any = false;
        for id in find_pairs(&state.config.data_dir) {
            match media::legacy_exists(&state.db, &id).await {
                Ok(false) => {
                    any = true;
                    break;
                }
                Ok(true) => {}
                Err(e) => {
                    tracing::warn!("legacy lookup failed: {e}");
                    return;
                }
            }
        }
        any
    };
    if pending {
        if let Err(e) = import(state).await {
            tracing::error!("legacy import failed: {e:#}");
        }
    }
}

pub async fn import(state: &AppState) -> anyhow::Result<ImportReport> {
    let mut report = ImportReport::default();
    sweep_temps(state).await;
    let mut pending = Vec::new();
    for id in find_pairs(&state.config.data_dir) {
        if !media::legacy_exists(&state.db, &id).await? {
            pending.push(id);
        }
    }
    tracing::info!("legacy import: {} pairs pending", pending.len());
    for (i, id) in pending.iter().enumerate() {
        if i > 0 && i % 100 == 0 {
            tracing::info!("legacy import: {i}/{} processed", pending.len());
        }
        match import_one(state, id).await {
            Ok(true) => report.imported += 1,
            Ok(false) => report.skipped += 1,
            Err(e) => {
                tracing::warn!("legacy import of {id} failed: {e:#}");
                report.skipped += 1;
            }
        }
    }
    tracing::info!(
        "legacy import: {} imported, {} skipped",
        report.imported,
        report.skipped
    );
    Ok(report)
}

fn upload_dir(state: &AppState) -> PathBuf {
    state
        .config
        .data_dir
        .join(safe_folder(&state.settings.get().upload_folder))
}

/// Removes temp links left by a crashed earlier run.
async fn sweep_temps(state: &AppState) {
    let Ok(mut rd) = tokio::fs::read_dir(upload_dir(state)).await else {
        return;
    };
    while let Ok(Some(e)) = rd.next_entry().await {
        if e.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            let _ = tokio::fs::remove_file(e.path()).await;
        }
    }
}

/// An earlier run that died between claiming a name and inserting the row leaves
/// a row-less copy in the upload folder; find it by size, then content id.
async fn find_orphan(dir: &Path, size: u64, id: &str) -> Option<PathBuf> {
    let mut rd = tokio::fs::read_dir(dir).await.ok()?;
    while let Ok(Some(e)) = rd.next_entry().await {
        let Ok(meta) = tokio::fs::symlink_metadata(e.path()).await else {
            continue;
        };
        if !meta.file_type().is_file()
            || meta.len() != size
            || e.file_name().to_string_lossy().starts_with(".incoming-")
        {
            continue;
        }
        let p = e.path();
        let q = p.clone();
        if let Ok(Ok(found)) = tokio::task::spawn_blocking(move || media_id(&q)).await {
            if found == id {
                return Some(p);
            }
        }
    }
    None
}

/// Extension from the sidecar name, else from the content type.
fn extension_for(sc: &Sidecar) -> Option<String> {
    let from_name = sc.name.as_deref().and_then(|n| {
        Path::new(&basename(n))
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
    });
    if from_name.is_some() {
        return from_name;
    }
    let mime: mime_guess::mime::Mime = sc.content_type.as_deref()?.parse().ok()?;
    let exts = mime_guess::get_mime_extensions_str(mime.essence_str())?;
    exts.first().map(|e| e.to_string())
}

async fn mark_unimportable(state: &AppState, old_id: &str, why: &str) -> anyhow::Result<bool> {
    tracing::warn!("legacy {old_id}: {why}, leaving files");
    media::insert_legacy(&state.db, old_id, UNIMPORTABLE).await?;
    Ok(false)
}

/// Ok(true) = imported, Ok(false) = skipped.
async fn import_one(state: &AppState, old_id: &str) -> anyhow::Result<bool> {
    let data_dir = &state.config.data_dir;
    let raw = data_dir.join(old_id);
    let sidecar_path = data_dir.join(format!("{old_id}.json"));

    let sc: Sidecar = match serde_json::from_slice(&tokio::fs::read(&sidecar_path).await?) {
        Ok(sc) => sc,
        Err(e) => return mark_unimportable(state, old_id, &format!("bad sidecar ({e})")).await,
    };
    let Some(ext) = extension_for(&sc) else {
        return mark_unimportable(state, old_id, "no usable name or content type").await;
    };
    let name_src = match &sc.name {
        Some(n) => n.clone(),
        None => format!("{old_id}.{ext}"),
    };
    let head = {
        use tokio::io::AsyncReadExt;
        let mut f = tokio::fs::File::open(&raw).await?;
        let mut buf = vec![0u8; SNIFF_BYTES];
        let mut n = 0;
        while n < SNIFF_BYTES {
            let r = f.read(&mut buf[n..]).await?;
            if r == 0 {
                break;
            }
            n += r;
        }
        buf.truncate(n);
        buf
    };
    let Some(kind) = kind_for(&ext, &head) else {
        return mark_unimportable(state, old_id, &format!("unsupported type .{ext}")).await;
    };
    let meta = tokio::fs::symlink_metadata(&raw).await?;
    let created_at: Option<DateTime<Utc>> = meta.modified().ok().map(DateTime::<Utc>::from);

    let id = {
        let p = raw.clone();
        tokio::task::spawn_blocking(move || media_id(&p)).await??
    };

    if media::get(&state.db, &id).await?.is_some() {
        // Same clip already in the library: just map the old link to it.
        media::insert_legacy(&state.db, old_id, &id).await?;
        let _ = tokio::fs::remove_file(&sidecar_path).await;
        let _ = tokio::fs::remove_file(&raw).await;
        return Ok(false);
    }

    let mut slug = None;
    for cand in slug_candidates(&id) {
        if !media::slug_exists(&state.db, &cand).await? {
            slug = Some(cand);
            break;
        }
    }
    let slug = slug.ok_or_else(|| anyhow::anyhow!("no free slug for {id}"))?;

    let dir = upload_dir(state);
    tokio::fs::create_dir_all(&dir).await?;
    let dir = tokio::fs::canonicalize(&dir).await?;
    let (final_path, adopted) = match find_orphan(&dir, meta.len(), &id).await {
        Some(p) => (p, true),
        None => {
            // Link the raw file to a temp name so the original stays until the row exists.
            let mut tmp = [0u8; 8];
            rand::rng().fill_bytes(&mut tmp);
            let temp: PathBuf = dir.join(format!("{TEMP_PREFIX}{}", hex::encode(tmp)));
            if tokio::fs::hard_link(&raw, &temp).await.is_err() {
                tokio::fs::copy(&raw, &temp).await?;
            }
            let name = sanitise_name(&name_src, &id, &ext);
            match claim_path(&dir, &name, &temp).await {
                Ok(p) => (p, false),
                Err(e) => {
                    let _ = tokio::fs::remove_file(&temp).await;
                    return Err(e.into());
                }
            }
        }
    };
    let final_name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let new = NewMedia {
        id: id.clone(),
        slug,
        kind,
        path: final_path.to_string_lossy().into_owned(),
        source: Source::Upload,
        folder: String::new(),
        extension: ext,
        size: meta.len() as i64,
        title: Path::new(&final_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        public: sc.listed,
        recorded_at: recorded_at_from_name(&final_name),
        created_at,
    };
    if let Err(e) = media::insert(&state.db, &new).await {
        if !adopted {
            let _ = tokio::fs::remove_file(&final_path).await;
        }
        return Err(e);
    }
    // A failure here leaves the old pair in place; the next run takes the duplicate path.
    media::insert_legacy(&state.db, old_id, &id).await?;
    let _ = tokio::fs::remove_file(&sidecar_path).await;
    let _ = tokio::fs::remove_file(&raw).await;
    if let Some(row) = media::get(&state.db, &id).await? {
        crate::jobs::enqueue_for_new_media(state, &row).await;
    }
    Ok(true)
}
