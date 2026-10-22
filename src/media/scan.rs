//! Library scanner: discovers media in the configured folders, relinks moved
//! files, hides vanished ones, and runs on a schedule and on file changes.
//!
//! Scanned files are always inserted link-only (`public = false`) so a whole
//! recordings folder is never published by accident. All stored paths are
//! canonical; every discovered file is canonicalised and must lie under a
//! canonical root before it is considered.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use chrono::Utc;
use serde::Serialize;
use walkdir::WalkDir;

use crate::app::AppState;
use crate::db::media::{self, NewMedia};
use crate::jobs::{enqueue_for_new_media, Event, ScanSummary};
use crate::media::filename_date::recorded_at_from_name;
use crate::media::ids::{media_id, slug_candidates};
use crate::media::kind::{kind_for, Source};
use crate::web::upload::safe_folder;

const GROWING_WAIT: Duration = Duration::from_secs(2);
const RECENT_MTIME: Duration = Duration::from_secs(10);
const SNIFF: usize = 8 * 1024;
const WATCH_DEBOUNCE: Duration = Duration::from_secs(3);
const RETRY_AFTER_GROWING: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ScanReport {
    pub found: u32,
    pub relinked: u32,
    pub missing: u32,
    pub skipped_growing: u32,
}

/// Single-flight state shared through `AppState`.
#[derive(Default)]
pub struct ScanControl {
    lock: tokio::sync::Mutex<()>,
    rerun: AtomicBool,
    last: Mutex<ScanReport>,
}

struct Root {
    path: PathBuf,
    exts: Vec<String>,
}

struct Candidate {
    path: PathBuf,
    root: usize,
    ext: String,
    size: u64,
}

pub struct Scanner {
    state: AppState,
}

fn norm_exts(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .collect()
}

fn warn_once(path: &Path) {
    static WARNED: std::sync::OnceLock<Mutex<HashSet<PathBuf>>> = std::sync::OnceLock::new();
    let set = WARNED.get_or_init(Default::default);
    let first = set
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_path_buf());
    if first {
        tracing::warn!("scan folder {} does not exist; skipping", path.display());
    }
}

impl Scanner {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Canonical roots with their extension allowlists. Missing roots are skipped.
    async fn roots(&self) -> Vec<Root> {
        let settings = self.state.settings.get();
        let video = norm_exts(&settings.video_exts);
        let image = norm_exts(&settings.image_exts);
        let both: Vec<String> = video.iter().chain(image.iter()).cloned().collect();
        let upload = self
            .state
            .config
            .data_dir
            .join(safe_folder(&settings.upload_folder));
        let mut wanted: Vec<(PathBuf, Vec<String>, bool)> = vec![(upload, both, false)];
        for d in &self.state.config.video_dirs {
            wanted.push((d.clone(), video.clone(), true));
        }
        for d in &self.state.config.image_dirs {
            wanted.push((d.clone(), image.clone(), true));
        }
        let mut roots: Vec<Root> = Vec::new();
        for (dir, exts, configured) in wanted {
            match tokio::fs::canonicalize(&dir).await {
                Ok(path) => match roots.iter_mut().find(|r| r.path == path) {
                    Some(existing) => {
                        for e in exts {
                            if !existing.exts.contains(&e) {
                                existing.exts.push(e);
                            }
                        }
                    }
                    None => roots.push(Root { path, exts }),
                },
                Err(_) if configured => warn_once(&dir),
                Err(_) => tracing::debug!("upload folder {} does not exist yet", dir.display()),
            }
        }
        roots
    }

    /// One full pass over every root.
    pub async fn run_once(&self) -> anyhow::Result<ScanReport> {
        let state = &self.state;
        let mut report = ScanReport::default();
        let roots = self.roots().await;
        let derived = tokio::fs::canonicalize(state.config.data_dir.join("derived"))
            .await
            .ok();

        let walk_roots: Vec<(PathBuf, Vec<String>)> = roots
            .iter()
            .map(|r| (r.path.clone(), r.exts.clone()))
            .collect();
        let found_files =
            tokio::task::spawn_blocking(move || walk(&walk_roots, derived.as_deref())).await?;
        let seen: HashSet<PathBuf> = found_files.iter().map(|c| c.path.clone()).collect();

        // Known files need no further checks beyond re-marking them available.
        let mut pending: Vec<Candidate> = Vec::new();
        for cand in found_files {
            let path = cand.path.to_string_lossy().into_owned();
            if let Some(m) = media::get_by_path(&state.db, &path).await? {
                if !m.available {
                    media::set_available(&state.db, &m.id, true).await?;
                }
                continue;
            }
            if media::is_tombstoned(&state.db, &path).await? {
                continue;
            }
            pending.push(cand);
        }

        // Growing-file rule: the walk's stat is the first one, re-stat after one shared wait.
        if !pending.is_empty() {
            tokio::time::sleep(GROWING_WAIT).await;
        }
        for cand in pending {
            let Ok(meta) = tokio::fs::metadata(&cand.path).await else {
                continue;
            };
            let recent = meta
                .modified()
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age < RECENT_MTIME);
            if meta.len() != cand.size || recent {
                report.skipped_growing += 1;
                continue;
            }
            if let Err(e) = self.ingest(&cand, meta.len(), &roots, &mut report).await {
                tracing::warn!("scan: {}: {e:#}", cand.path.display());
            }
        }

        report.missing = self.mark_missing(&roots, &seen).await?;
        Ok(report)
    }

    async fn ingest(
        &self,
        cand: &Candidate,
        size: u64,
        roots: &[Root],
        report: &mut ScanReport,
    ) -> anyhow::Result<()> {
        let state = &self.state;
        let mut head = vec![0u8; SNIFF];
        let n = {
            let mut f = tokio::fs::File::open(&cand.path).await?.into_std().await;
            tokio::task::spawn_blocking(move || -> std::io::Result<Vec<u8>> {
                let n = f.read(&mut head)?;
                head.truncate(n);
                Ok(head)
            })
            .await??
        };
        let Some(kind) = kind_for(&cand.ext, &n) else {
            tracing::debug!(
                "scan: {} is not a supported media file",
                cand.path.display()
            );
            return Ok(());
        };

        let hash_path = cand.path.clone();
        let id = tokio::task::spawn_blocking(move || media_id(&hash_path)).await??;
        let path = cand.path.to_string_lossy().into_owned();
        let root = &roots[cand.root].path;
        let folder = cand
            .path
            .parent()
            .and_then(|p| p.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        if let Some(existing) = media::get(&state.db, &id).await? {
            if tokio::fs::try_exists(&existing.path).await.unwrap_or(false) {
                tracing::debug!("scan: {path} duplicates {}; skipped", existing.path);
                return Ok(());
            }
            match existing.source {
                Source::Scan => media::relocate(&state.db, &id, &path, &folder).await?,
                Source::Upload => media::relink(&state.db, &id, &path).await?,
            }
            report.relinked += 1;
            return Ok(());
        }

        let mut slug = None;
        for cand_slug in slug_candidates(&id) {
            if !media::slug_exists(&state.db, &cand_slug).await? {
                slug = Some(cand_slug);
                break;
            }
        }
        let slug = slug.ok_or_else(|| anyhow::anyhow!("no free slug for {id}"))?;
        let name = cand
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let new = NewMedia {
            id: id.clone(),
            slug,
            kind,
            path,
            source: Source::Scan,
            folder,
            extension: cand.ext.clone(),
            size: size as i64,
            title: cand
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            public: false,
            recorded_at: recorded_at_from_name(&name),
            created_at: None,
        };
        media::insert(&state.db, &new).await?;
        if let Some(row) = media::get(&state.db, &id).await? {
            enqueue_for_new_media(state, &row).await;
        }
        report.found += 1;
        Ok(())
    }

    /// Rows under this pass's roots whose file is gone become unavailable.
    /// Rows under skipped (missing) roots are never touched.
    async fn mark_missing(&self, roots: &[Root], seen: &HashSet<PathBuf>) -> anyhow::Result<u32> {
        let mut missing = 0;
        for row in media::list_paths(&self.state.db).await? {
            let p = Path::new(&row.path);
            if !row.available || seen.contains(p) || !roots.iter().any(|r| p.starts_with(&r.path)) {
                continue;
            }
            if !tokio::fs::try_exists(p).await.unwrap_or(true) {
                media::set_available(&self.state.db, &row.id, false).await?;
                missing += 1;
            }
        }
        Ok(missing)
    }
}

/// Blocking directory walk. Returns candidate files whose canonical path is
/// under a canonical root, skipping hidden entries and the derived directory.
fn walk(roots: &[(PathBuf, Vec<String>)], derived: Option<&Path>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let mut taken: HashSet<PathBuf> = HashSet::new();
    for (idx, (root, exts)) in roots.iter().enumerate() {
        let walker = WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| {
                let hidden = e.depth() > 0 && e.file_name().to_string_lossy().starts_with('.');
                let is_derived = e.file_type().is_dir()
                    && derived
                        .is_some_and(|d| std::fs::canonicalize(e.path()).is_ok_and(|c| c == d));
                !hidden && !is_derived
            });
        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!("scan walk: {e}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(ext) = entry
                .path()
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
            else {
                continue;
            };
            if !exts.contains(&ext) {
                continue;
            }
            let Ok(canon) = std::fs::canonicalize(entry.path()) else {
                continue;
            };
            if !canon.starts_with(root) || derived.is_some_and(|d| canon.starts_with(d)) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&canon) else {
                continue;
            };
            if !taken.insert(canon.clone()) {
                continue;
            }
            out.push(Candidate {
                path: canon,
                root: idx,
                ext,
                size: meta.len(),
            });
        }
    }
    out
}

/// Run a scan now, or ask the running one to go around once more. Concurrent
/// callers never run two walks; a caller that waited returns the report of the
/// pass that covered its request.
pub async fn request_scan(state: &AppState) -> anyhow::Result<ScanReport> {
    let ctl = &state.scan;
    let (_guard, waited) = match ctl.lock.try_lock() {
        Ok(g) => (g, false),
        Err(_) => {
            ctl.rerun.store(true, Ordering::SeqCst);
            (ctl.lock.lock().await, true)
        }
    };
    if waited && !ctl.rerun.load(Ordering::SeqCst) {
        return Ok(*ctl.last.lock().unwrap_or_else(|e| e.into_inner()));
    }
    loop {
        ctl.rerun.store(false, Ordering::SeqCst);
        let report = Scanner::new(state.clone()).run_once().await?;
        *ctl.last.lock().unwrap_or_else(|e| e.into_inner()) = report;
        let summary = ScanSummary {
            at: Utc::now(),
            found: report.found,
        };
        *state.last_scan.write().await = Some(summary.clone());
        let _ = state.events.send(Event::Scan(summary));
        tracing::info!(
            "scan: {} new, {} relinked, {} missing, {} still growing",
            report.found,
            report.relinked,
            report.missing,
            report.skipped_growing
        );
        if !ctl.rerun.load(Ordering::SeqCst) {
            return Ok(report);
        }
    }
}

/// `request_scan` for background triggers: files skipped as still growing get
/// another look once they have had time to settle.
async fn background_scan(state: &AppState) {
    for _ in 0..5 {
        match request_scan(state).await {
            Ok(r) if r.skipped_growing > 0 => tokio::time::sleep(RETRY_AFTER_GROWING).await,
            Ok(_) => return,
            Err(e) => {
                tracing::warn!("scan failed: {e:#}");
                return;
            }
        }
    }
}

/// Startup pass, periodic scheduler and file watcher. Never fatal.
pub fn spawn_background(state: &AppState) {
    let s = state.clone();
    tokio::spawn(async move {
        // Legacy import first so the startup scan sees its files.
        crate::legacy::import_if_needed(&s).await;
        background_scan(&s).await;
        loop {
            let minutes = s.settings.get().scan_interval_minutes;
            if minutes == 0 {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }
            tokio::time::sleep(Duration::from_secs(minutes.max(1) * 60)).await;
            background_scan(&s).await;
        }
    });
    let s = state.clone();
    tokio::spawn(async move { watch(s).await });
}

async fn watch(state: AppState) {
    use notify::{RecursiveMode, Watcher};
    let roots: Vec<PathBuf> = Scanner::new(state.clone())
        .roots()
        .await
        .into_iter()
        .map(|r| r.path)
        .collect();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut watcher =
        match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                let relevant = !matches!(ev.kind, notify::EventKind::Access(_))
                    && ev.paths.iter().any(|p| {
                        !p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                    });
                if relevant {
                    let _ = tx.send(());
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("file watcher unavailable: {e}");
                return;
            }
        };
    let mut watching = 0;
    for root in &roots {
        match watcher.watch(root, RecursiveMode::Recursive) {
            Ok(()) => watching += 1,
            Err(e) => tracing::warn!("cannot watch {}: {e}", root.display()),
        }
    }
    if watching == 0 {
        return;
    }
    while rx.recv().await.is_some() {
        while let Ok(Some(())) = tokio::time::timeout(WATCH_DEBOUNCE, rx.recv()).await {}
        background_scan(&state).await;
    }
}
