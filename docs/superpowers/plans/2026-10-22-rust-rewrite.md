# FTHR Clips Server Rust Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Python service with a single Rust binary that serves a Fireshare-class clip and image library in the FTHR Clips 2.0 look, keeping the desktop app's upload contract intact.

**Architecture:** One axum binary with SQLite (sqlx) for every piece of state, Askama templates plus embedded static assets for the UI, and a DB-backed job queue whose workers drive ffmpeg and ffprobe as subprocesses. Scanning, serving and admin never depend on ffmpeg being present. Tasks are ordered so the server is runnable and tested after every task.

**Tech Stack:** Rust stable (edition 2021), axum 0.8, tokio, tower-http 0.6, sqlx 0.8 (sqlite, bundled), askama 0.14, rust-embed 8, xxhash-rust (xxh3), argon2, notify 8, image 0.25, reqwest 0.12 (rustls), rss 2, chrono 0.4, clap 4, tracing.

**Spec:** `docs/superpowers/specs/2026-10-22-rust-rewrite-design.md`

## Global Constraints

- License: GPL-3.0-only. `LICENSE` holds the full GPL-3.0 text; `Cargo.toml` has `license = "GPL-3.0-only"`.
- Upload contract is frozen: `POST /upload` multipart field `clip`, header `Authorization: Bearer <token>`, 2xx JSON with `url`; `HEAD /upload` answers 200 with a valid bearer and 401 without. Errors 401 / 413 / 415 / 429.
- Env var names that must keep working: `UPLOAD_TOKEN`, `ADMIN_PASSWORD`, `STORAGE_DIR` (alias of `DATA_DIR`), `PUBLIC_BASE_URL`, `MAX_UPLOAD_BYTES` (default 524288000), `BIND_HOST` (default 127.0.0.1), `PORT` (default 8080).
- No OpenSSL, no C deps beyond bundled SQLite. Must build for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
- Serving must never require ffmpeg. The process starts and serves with ffmpeg absent.
- Only DB-recorded paths are ever opened for serving; never request-derived paths.
- All user strings reach HTML through Askama escaping; JS never assigns user data via `innerHTML`.
- Colour tokens, fonts and dimensions are those in the spec's Frontend section, copied verbatim.
- Commit after every task on branch `rust-rewrite`. Commit messages end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Review Focus

1. Upload whose filename has no extension or an unknown one (e.g. `clip`, `clip.exe`): expect 415 with no file left on disk. Test in Task 6.
2. `Range: bytes=999999999-` past the end of a media file: expect 416, never a 500. Test in Task 7.
3. A scanned file that is still being written (size changing): expect it skipped this scan and picked up on the next. Test in Task 16.
4. `X-Forwarded-For` from a peer that is not in `TRUSTED_PROXIES`: expect it ignored, the view keyed by the socket IP, and the cookie not marked Secure from a forged `X-Forwarded-Proto`. Test in Task 5.
5. ffmpeg absent at startup: expect the server to start, the library and clip pages to render with placeholder posters, and jobs to remain `queued`. Test in Task 15.

---

## File structure

```
Cargo.toml                 crate metadata, deps, release profile
build.rs                   none (askama needs none)
LICENSE                    GPL-3.0 text
migrations/0001_init.sql   full schema (one migration for the rewrite)
src/main.rs                clap CLI: serve | scan | import-legacy | hash-password | check
src/config.rs              Config: env + toml + defaults, TrustedProxies, PublicBase
src/app.rs                 AppState, build_router(), serve()
src/db/mod.rs              Db (SqlitePool), open(), migrate
src/db/media.rs            Media row type + queries
src/db/tags.rs             Tag row type + queries
src/db/sessions.rs         Session row type + queries
src/db/settings.rs         key/value JSON settings
src/db/tokens.rs           UploadToken row + queries
src/db/jobs.rs             Job + Rendition rows + queue queries
src/db/views.rs            view insert + count
src/settings.rs            Settings (effective, DB overrides over Config), SettingsStore
src/media/ids.rs           media_id(path), slug_candidates(id)
src/media/kind.rs          MediaKind, kind_for(extension, head_bytes)
src/media/playable.rs      is_browser_playable(container, vcodec, acodec)
src/media/filename_date.rs recorded_at_from_name(name)
src/media/scan.rs          Scanner: walk, identify, insert/relink/unavailable, enqueue
src/media/paths.rs         DerivedPaths: derived dir and file names per media
src/ffmpeg/mod.rs          Ffmpeg: locate binaries, version, run_with_progress
src/ffmpeg/probe.rs        ffprobe JSON -> ProbeInfo
src/ffmpeg/encoders.rs     detect_encoder() -> Encoder
src/jobs/mod.rs            JobKind, JobEvent, EventBus (tokio broadcast)
src/jobs/worker.rs         Worker pool: claim, run, retry, cancel, timeout
src/jobs/poster.rs         poster + preview jobs
src/jobs/thumb.rs          image thumb job (image crate)
src/jobs/transcode.rs      transcode job + rendition bookkeeping
src/web/mod.rs             route tree assembly
src/web/security.rs        headers layer, RateLimiter, client_ip(), csrf check
src/web/upload.rs          HEAD/POST /upload
src/web/media_files.rs     GET /media/{slug}/{file}
src/web/library.rs         GET /, /images, /api/library
src/web/clip.rs            GET /w/{slug}, /i/{slug}, view counting
src/web/feed.rs            GET /feed.rss
src/web/legacy.rs          GET /files/{old_id}
src/web/admin/mod.rs       admin router, auth extractor
src/web/admin/auth.rs      login/logout, session cookie, csrf
src/web/admin/pages.rs     GET /admin and tab pages
src/web/admin/api_media.rs media PATCH/DELETE/bulk/transcode
src/web/admin/api_tags.rs  tags CRUD
src/web/admin/api_settings.rs GET/PUT settings
src/web/admin/api_tokens.rs tokens CRUD
src/web/admin/api_jobs.rs  jobs list/cancel/retry, scan, stats, SSE events
src/webhooks.rs            Discord + generic webhook on first public
src/legacy.rs              import_legacy(data_dir, db)
templates/*.html           base, library, clip, image, login, admin/*.html
static/app.css static/app.js static/admin.js static/fonts/* static/icons.svg static/logo.png
tests/common/mod.rs        TestServer harness
tests/*.rs                 one file per task area
Dockerfile docker-compose.yml .github/workflows/{ci,release}.yml README.md
```

---

### Task 1: Delete the Python service and scaffold the Rust crate

**Files:**
- Delete: `fthr_server.py`, `tests/`, `pyproject.toml`, `requirements.txt`, `requirements-dev.txt`, `Dockerfile`
- Create: `Cargo.toml`, `LICENSE`, `src/main.rs`, `src/config.rs`, `src/app.rs`, `src/web/mod.rs`, `tests/common/mod.rs`, `tests/health.rs`
- Modify: `.gitignore` (add `target/`, `.superpowers/`, `*.db`, `*.db-wal`, `*.db-shm`), `README.md` (replace body with a two-line "rewrite in progress" note; full README is Task 22)

**Interfaces:**
- Produces: `Config` with fields `bind_host: String`, `port: u16`, `data_dir: PathBuf`, `video_dirs: Vec<PathBuf>`, `image_dirs: Vec<PathBuf>`, `upload_token: Option<String>`, `admin_password: Option<String>`, `admin_password_hash: Option<String>`, `public_base_url: Option<String>`, `max_upload_bytes: u64`, `trusted_proxies: Vec<ipnet::IpNet>`, `scan_interval_minutes: u64`, `transcoding: TranscodeMode` (`Off|Compat|On`, default `Compat`), `encoder: EncoderChoice` (`Auto|X264|Nvenc|Vaapi|Qsv|VideoToolbox`), `transcode_workers: usize` (1), `transcode_timeout_minutes: u64` (60), `ffmpeg_path: Option<PathBuf>`, `ffprobe_path: Option<PathBuf>`, `discord_webhook_url: Option<String>`, `webhook_url: Option<String>`, `site_title: String` ("FTHR Clips"), `log_level: String`.
- Produces: `Config::load() -> anyhow::Result<Config>` (defaults < `{data_dir}/config.toml` < env; `STORAGE_DIR` accepted as `DATA_DIR` alias) and `Config::from_env_map(vars: &HashMap<String,String>, toml: Option<&str>) -> anyhow::Result<Config>` for tests.
- Produces: `app::build_router(state: AppState) -> axum::Router`, `AppState { config: Arc<Config>, ... }` (grows per task), `app::serve(config) -> anyhow::Result<()>`.
- Produces: `tests/common/mod.rs`: `TestServer::spawn(env: &[(&str,&str)]) -> TestServer` with `base: String`, `client: reqwest::Client`, `data_dir: tempfile::TempDir`; `fn url(&self, path:&str) -> String`. Spawns `build_router` on `127.0.0.1:0`.

- [ ] **Step 1: Branch and delete**

```bash
git checkout -b rust-rewrite
git rm -r fthr_server.py tests pyproject.toml requirements.txt requirements-dev.txt Dockerfile
```

- [ ] **Step 2: Write `Cargo.toml`**

Package `fthr-clips-server`, version `2.0.0-alpha.1`, edition 2021, `license = "GPL-3.0-only"`. Dependencies: `axum = { version = "0.8", features = ["multipart", "macros"] }`, `tokio = { features = ["full"] }`, `tower`, `tower-http = { features = ["compression-gzip", "compression-br", "trace", "timeout", "set-header", "fs"] }`, `sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio", "sqlite", "migrate", "chrono"] }`, `askama = "0.14"`, `rust-embed = "8"`, `xxhash-rust = { features = ["xxh3"] }`, `argon2`, `subtle`, `sha2`, `rand`, `notify = "8"`, `image = { version = "0.25", default-features = false, features = ["png", "jpeg", "webp", "gif"] }`, `serde`, `serde_json`, `toml`, `clap = { features = ["derive"] }`, `tracing`, `tracing-subscriber = { features = ["env-filter"] }`, `reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }`, `rss = "2"`, `chrono = { features = ["serde"] }`, `anyhow`, `thiserror`, `walkdir`, `tokio-util = { features = ["io", "rt"] }` (CancellationToken), `sysinfo = { default-features = false, features = ["disk"] }`, `futures`, `http-body-util`, `mime_guess`, `infer`, `hex`, `ipnet`, `base64`, `urlencoding`, `tokio-stream = { features = ["sync"] }`, `async-stream`. Dev: `tempfile`, `reqwest` with `multipart` and `stream`. `[profile.release] lto = "thin", codegen-units = 1, strip = true`.

- [ ] **Step 3: Write `LICENSE`** with the verbatim GPL-3.0 text (fetch from https://www.gnu.org/licenses/gpl-3.0.txt).

- [ ] **Step 4: Write the failing test `tests/health.rs`**

```rust
mod common;
#[tokio::test]
async fn healthz_returns_ok() {
    let s = common::TestServer::spawn(&[("UPLOAD_TOKEN", "t")]).await;
    let r = s.client.get(s.url("/healthz")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<serde_json::Value>().await.unwrap(), serde_json::json!({"status":"ok"}));
}
#[test]
fn config_storage_dir_alias_and_defaults() {
    let mut v = std::collections::HashMap::new();
    v.insert("STORAGE_DIR".to_string(), "/tmp/x".to_string());
    let c = fthr_clips_server::config::Config::from_env_map(&v, None).unwrap();
    assert_eq!(c.data_dir, std::path::PathBuf::from("/tmp/x"));
    assert_eq!(c.port, 8080);
    assert_eq!(c.max_upload_bytes, 524_288_000);
    assert!(matches!(c.transcoding, fthr_clips_server::config::TranscodeMode::Compat));
}
```

The crate needs a `src/lib.rs` exporting modules so tests can import it; `main.rs` calls into the lib.

- [ ] **Step 5: Run to verify failure**

Run: `cargo test --test health`
Expected: compile error, `common` and `config` missing.

- [ ] **Step 6: Implement `config.rs`, `app.rs` with `/healthz`, `lib.rs`, `main.rs` (`serve` subcommand only for now), `tests/common/mod.rs`**

`main.rs`: clap `Cli { #[command(subcommand)] cmd: Option<Cmd> }`, `Cmd::Serve` default; initialise `tracing_subscriber` with `LOG_LEVEL`. `Config::load` fails with a clear message if `UPLOAD_TOKEN` is unset and no `config.toml` exists (keeps today's behaviour); `from_env_map` does not enforce that.

- [ ] **Step 7: Run tests**

Run: `cargo test`
Expected: 2 passed.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "rewrite: remove Python service, scaffold Rust crate"
```

---

### Task 2: Database and schema

**Files:**
- Create: `migrations/0001_init.sql`, `src/db/mod.rs`, `src/db/media.rs`, `src/db/tags.rs`, `src/db/sessions.rs`, `src/db/settings.rs`, `src/db/tokens.rs`, `src/db/jobs.rs`, `src/db/views.rs`, `tests/db.rs`
- Modify: `src/app.rs` (AppState gains `db: Db`; `serve` opens `{data_dir}/library.db`), `tests/common/mod.rs`

**Interfaces:**
- Produces: `Db(SqlitePool)`; `Db::open(path: &Path) -> Result<Db>` (creates parent dir, `journal_mode=WAL`, `foreign_keys=ON`, runs `sqlx::migrate!()`); `Db::open_memory()` for tests.
- Produces `MediaKind { Video, Image }` (lives in `media/kind.rs`, re-exported), `Source { Scan, Upload }`.
- Produces `Media` (sqlx `FromRow`): `id: String, slug: String, kind: MediaKind, path: String, source: Source, folder: String, extension: String, size: i64, available: bool, title: String, description: String, public: bool, created_at: DateTime<Utc>, updated_at: DateTime<Utc>, recorded_at: Option<DateTime<Utc>>, duration_ms: Option<i64>, width: Option<i64>, height: Option<i64>, codec_video: Option<String>, codec_audio: Option<String>, container: Option<String>, fps: Option<f64>, bitrate: Option<i64>, probe_json: Option<String>, has_poster: bool, has_preview: bool, has_thumb: bool, processing_state: String ("pending"|"ready"|"failed"), processing_error: Option<String>`.
- Produces `NewMedia { id, slug, kind, path, source, folder, extension, size, title, public, recorded_at }` and queries in `db/media.rs`: `insert(&Db, &NewMedia) -> Result<()>`, `get(&Db, id) -> Result<Option<Media>>`, `get_by_slug(&Db, slug) -> Result<Option<Media>>`, `slug_exists(&Db, slug) -> Result<bool>`, `get_by_path(&Db, path) -> Result<Option<Media>>`, `set_available(&Db, id, bool)`, `relink(&Db, id, new_path)`, `update_details(&Db, id, title, description)`, `set_public(&Db, ids: &[String], public: bool)`, `set_probe(&Db, id, &ProbeFields)`, `set_flags(&Db, id, has_poster: Option<bool>, has_preview: Option<bool>, has_thumb: Option<bool>)`, `set_processing(&Db, id, state, error)`, `delete(&Db, id)`, `list(&Db, &ListQuery) -> Result<Vec<Media>>` where `ListQuery { kind: Option<MediaKind>, public_only: bool, q: Option<String>, tag: Option<String>, sort: Sort, page: u32, per_page: u32 }` and `Sort { Newest, Oldest, MostViewed, Title, Longest }` (`list` joins view counts and returns `MediaWithViews { media, views: i64, tags: Vec<Tag> }`), `count(&Db, kind, public_only) -> i64`, `all_ids(&Db) -> Vec<String>`.
- Produces `Tag { id: i64, name: String, colour: String }`; `db/tags.rs`: `create(name, colour)`, `rename`, `recolour`, `delete`, `list_all`, `attach(media_id, tag_id)`, `detach`, `for_media(media_id) -> Vec<Tag>`, `attach_by_name(media_ids, name)` (creates if missing, default colour `#20e3a0`).
- Produces `db/sessions.rs`: `Session { id, csrf, created_at, expires_at }`; `create(ttl) -> Session`, `get_valid(id) -> Option<Session>` (also bumps expiry), `delete(id)`, `purge_expired()`.
- Produces `db/settings.rs`: `get_json(key) -> Option<serde_json::Value>`, `set_json(key, value)`, `all() -> HashMap<String, Value>`.
- Produces `db/tokens.rs`: `UploadToken { id, name, prefix, token_hash, created_at, last_used_at, revoked_at }`; `create(name, token_hash, prefix)`, `find_active_by_hash(hash)`, `touch(id)`, `revoke(id)`, `list()`.
- Produces `db/jobs.rs`: `Job { id: i64, media_id, kind: String, state: String ("queued"|"running"|"done"|"failed"|"cancelled"), attempts: i64, progress: f64, error: Option<String>, created_at, updated_at }`; `enqueue(media_id, kind) -> i64` (no-op if an identical queued/running job exists), `claim_next(kinds: &[&str]) -> Option<Job>` (atomic queued→running), `set_progress(id, f64)`, `finish(id)`, `fail(id, error)`, `cancel(id)`, `retry(id)` (failed→queued), `list_active() -> Vec<Job>`, `list_for_media(media_id)`. `Rendition { media_id, height: i64, path, size, state, progress, error }`; `rendition_upsert`, `renditions_for(media_id)`, `rendition_delete(media_id, height)`, `rendition_ready(media_id, height, path, size)`.
- Produces `db/views.rs`: `record(media_id, ip_hash) -> bool` (true if new), `count(media_id) -> i64`.
- Produces tables also: `legacy_id(old_id PK, media_id)`, `tombstone(id PK, path)` for Task 10, `webhook_sent(media_id PK)` for Task 20.

- [ ] **Step 1: Write failing tests `tests/db.rs`**

```rust
use fthr_clips_server::db::{self, Db, media::{NewMedia, ListQuery, Sort}};
use fthr_clips_server::media::kind::MediaKind;
#[tokio::test] async fn migrates_and_round_trips_media() {
    let db = Db::open_memory().await.unwrap();
    db::media::insert(&db, &NewMedia::test("a1", "slugA", MediaKind::Video, "/v/a.mp4")).await.unwrap();
    let m = db::media::get(&db, "a1").await.unwrap().unwrap();
    assert_eq!(m.slug, "slugA"); assert!(m.available); assert!(!m.public);
    assert_eq!(db::media::get_by_slug(&db, "slugA").await.unwrap().unwrap().id, "a1");
}
#[tokio::test] async fn list_filters_public_and_tag_and_search() { /* insert 3 rows: public+tag "x", public no tag, private; assert list(public_only) == 2, list(tag "x") == 1, list(q "Second") == 1 by title */ }
#[tokio::test] async fn views_dedupe_by_ip_hash() { /* record twice same hash -> count 1, second record returns false */ }
#[tokio::test] async fn jobs_enqueue_is_idempotent_and_claim_is_atomic() { /* enqueue same (media,kind) twice -> one row; claim_next returns it as running; claim_next again -> None */ }
#[tokio::test] async fn sessions_expire() { /* create with ttl 0s -> get_valid None */ }
```

`NewMedia::test(id, slug, kind, path)` is a `#[cfg(any(test, feature = "test-helpers"))]` helper; enable a `test-helpers` feature in `Cargo.toml` and use it from integration tests via `required-features` or just make it always-available `pub fn test(...)` documented as test-only.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test db`
Expected: compile errors, `db` module missing.

- [ ] **Step 3: Write `migrations/0001_init.sql`** with the tables listed in the Interfaces block plus `media_tag(media_id, tag_id, PRIMARY KEY)`, `view(media_id, ip_hash, first_seen, PRIMARY KEY(media_id, ip_hash))`, indexes on `media(slug)`, `media(path)`, `media(public, created_at)`, `job(state, kind)`.

- [ ] **Step 4: Implement `src/db/*.rs`** per the Interfaces block. Use `sqlx::query` / `query_as` with string SQL (no compile-time macros, so no DATABASE_URL at build). Search in `list` is `LOWER(title) LIKE '%q%' OR LOWER(description) LIKE ... OR id IN (SELECT media_id FROM media_tag JOIN tag ... WHERE LOWER(name) LIKE ...)`.

- [ ] **Step 5: Wire `Db` into `AppState` and `TestServer`** (`TestServer` opens `library.db` inside its temp dir, not memory, so file paths exercise the same code).

- [ ] **Step 6: Run tests**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add -A && git commit -m "db: sqlite schema and query layer"
```

---

### Task 3: Media identity, kind detection, slugs, playability, filename dates

**Files:**
- Create: `src/media/mod.rs`, `src/media/ids.rs`, `src/media/kind.rs`, `src/media/playable.rs`, `src/media/filename_date.rs`, `src/media/paths.rs`, `tests/media_ids.rs`

**Interfaces:**
- Produces `ids::media_id(path: &Path) -> io::Result<String>`: xxh3-128 over head 16 MiB, tail 16 MiB (whole file once when size ≤ 32 MiB, no double counting), then `size.to_le_bytes()`; lowercase hex, 32 chars.
- Produces `ids::slug_candidates(id: &str) -> [String; 3]`: base62 (alphabet `0-9A-Za-z`) of the high 64 bits (left-padded to 11), of the low 64 bits, and both concatenated.
- Produces `kind::MediaKind` (`Video|Image`), `kind::VIDEO_EXTS = ["mp4","m4v","mov","webm","mkv","avi"]`, `kind::IMAGE_EXTS = ["png","jpg","jpeg","webp","gif"]`, `kind::kind_for(extension: &str, head: &[u8]) -> Option<MediaKind>` (extension lookup, then `infer` sniff must agree on the broad type; `None` means 415/skip).
- Produces `playable::is_browser_playable(container: &str, vcodec: Option<&str>, acodec: Option<&str>) -> bool`: container in `mp4|m4v|mov|webm` and vcodec in `h264|vp9|av1` and acodec `None` or in `aac|opus|mp3|vorbis`.
- Produces `filename_date::recorded_at_from_name(name: &str) -> Option<DateTime<Utc>>` for patterns `YYYY-MM-DD HH-MM-SS`, `YYYY-MM-DD_HH-MM-SS`, `YYYYMMDD_HHMMSS`, `YYYY-MM-DD` anywhere in the stem. Treat as local naive time converted with `Local`.
- Produces `paths::DerivedPaths::new(data_dir: &Path, media_id: &str)` with `dir()`, `poster()` (`poster.jpg`), `preview()` (`preview.mp4`), `thumb()` (`thumb.jpg`), `rendition(height) ` (`{h}p.mp4`), `remove_all()`.

- [ ] **Step 1: Write failing tests `tests/media_ids.rs`**

```rust
#[test] fn media_id_differs_when_tail_differs() { /* write two 40 MiB temp files identical in first 20 MiB, different last byte; ids differ */ }
#[test] fn media_id_stable_for_identical_bytes() { /* two files same bytes -> same id; id.len()==32 */ }
#[test] fn slug_candidates_are_base62_and_11_chars() { let c = slug_candidates("00000000000000ff00000000000000ff"); assert_eq!(c[0].len(), 11); assert!(c[0].chars().all(|ch| ch.is_ascii_alphanumeric())); assert_eq!(c[2].len(), 22); }
#[test] fn kind_for_rejects_unknown_and_mismatch() { assert!(kind_for("exe", b"MZ").is_none()); assert!(kind_for("mp4", b"\x89PNG\r\n\x1a\n").is_none()); assert_eq!(kind_for("png", b"\x89PNG\r\n\x1a\n\0\0"), Some(MediaKind::Image)); }
#[test] fn playable_rules() { assert!(is_browser_playable("mp4", Some("h264"), Some("aac"))); assert!(!is_browser_playable("mkv", Some("h264"), Some("aac"))); assert!(!is_browser_playable("mp4", Some("hevc"), Some("aac"))); assert!(is_browser_playable("webm", Some("vp9"), None)); }
#[test] fn filename_dates() { assert!(recorded_at_from_name("Clip 2026-10-22 18-03-11.mp4").is_some()); assert!(recorded_at_from_name("20261022_180311_valorant.mkv").is_some()); assert!(recorded_at_from_name("ace.mp4").is_none()); }
```

- [ ] **Step 2: Run to verify failure** — `cargo test --test media_ids` fails to compile.
- [ ] **Step 3: Implement the five modules** per Interfaces. For the sniff, accept when `infer::get(head)` is `None` only if the extension is in the allowlist and head is shorter than 12 bytes (tiny test fixtures); otherwise the matcher type must be `Video` for video exts (mkv/webm report as `video/x-matroska`/`video/webm`, mov/mp4/m4v as `video/*`, avi as `video/x-msvideo`) and `Image` for image exts.
- [ ] **Step 4: Run tests** — `cargo test` all pass.
- [ ] **Step 5: Commit** — `git commit -am "media: ids, kind sniffing, playability, filename dates"`

---

### Task 4: Settings layer and public base URL

**Files:**
- Create: `src/settings.rs`, `tests/settings.rs`
- Modify: `src/app.rs` (AppState gains `settings: SettingsStore`)

**Interfaces:**
- Produces `Settings` (serde, all admin-editable, defaults from `Config`): `site_title`, `public_base_url: Option<String>`, `library_requires_login: bool` (false), `default_sort: Sort` (Newest), `upload_default_public: bool` (true, matches today's `listed: true`), `upload_folder: String` ("uploads"), `scan_interval_minutes`, `video_exts: Vec<String>`, `image_exts: Vec<String>`, `transcoding: TranscodeMode`, `encoder: EncoderChoice`, `ladder: Vec<u32>` ([720]), `transcode_workers`, `transcode_timeout_minutes`, `discord_webhook_url`, `webhook_url`, `webhooks_enabled: bool` (true when either URL set).
- Produces `SettingsStore::load(db, config) -> Result<SettingsStore>` (reads `setting` rows, each key overrides the Config-derived default), `get() -> Arc<Settings>` (lock-free snapshot), `update(db, patch: serde_json::Value) -> Result<Arc<Settings>>` (merges known keys, persists each, swaps snapshot), `instance_secret() -> [u8; 32]` (created on first load, stored under key `instance_secret`).
- Produces `settings::public_base(settings: &Settings, headers: &HeaderMap, peer: IpAddr, trusted: &[IpNet]) -> String`: configured `public_base_url` trimmed of trailing `/`; else scheme from `X-Forwarded-Proto` only when `peer` is trusted, else `http`; host from `Host`.

- [ ] **Step 1: Write failing tests** — `settings_defaults_come_from_config`, `settings_update_persists_and_overrides` (reopen store, value survives), `public_base_prefers_config`, `public_base_ignores_forwarded_proto_from_untrusted_peer`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement `settings.rs`** using `arc_swap`-style `RwLock<Arc<Settings>>`.
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "settings: db-backed overrides and public base url"`

---

### Task 5: Security primitives: headers, rate limiter, client IP, CSRF helper

**Files:**
- Create: `src/web/security.rs`, `tests/security.rs`
- Modify: `src/app.rs` (apply headers layer to the whole router; AppState gains `login_limiter`, `upload_limiter`)

**Interfaces:**
- Produces `security_headers(req, next)` middleware setting `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `X-Frame-Options: DENY` unless the request extensions carry `AllowFraming`, `Content-Security-Policy: default-src 'self'; img-src 'self' data:; media-src 'self'; style-src 'self'; script-src 'self'; font-src 'self'; frame-ancestors 'none'` (inline scripts are not used), `Strict-Transport-Security: max-age=31536000` only when `request_is_https`.
- Produces `RateLimiter::new(limit: u32, window: Duration)`, `check(&self, key: &str) -> bool` (sliding window, in-memory, prunes).
- Produces `client_ip(headers: &HeaderMap, peer: IpAddr, trusted: &[IpNet]) -> IpAddr` (rightmost `X-Forwarded-For` entry not in trusted; peer when peer untrusted).
- Produces `request_is_https(headers, peer, trusted) -> bool`.
- Produces `ip_hash(secret: &[u8;32], ip: IpAddr) -> String` (hex sha256 of secret‖ip string).
- Produces `csrf_ok(session: &Session, headers: &HeaderMap, form_token: Option<&str>) -> bool` constant-time.

- [ ] **Step 1: Write failing tests** — `headers_present_on_healthz` (nosniff, DENY, CSP, no HSTS over http), `rate_limiter_blocks_after_limit`, `client_ip_ignores_xff_when_peer_untrusted`, `client_ip_uses_rightmost_untrusted_hop_when_peer_trusted`, `request_is_https_ignores_forged_proto` (Review Focus 4).
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement `security.rs`** and apply the layer in `build_router`.
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "web: security headers, rate limiting, client ip"`

---

### Task 6: Upload endpoint (frozen contract)

**Files:**
- Create: `src/web/upload.rs`, `tests/upload.rs`
- Modify: `src/web/mod.rs`, `src/app.rs`

**Interfaces:**
- Consumes `ids::media_id`, `slug_candidates`, `kind_for`, `RateLimiter`, `public_base`, `db::media`, `db::tokens`.
- Produces routes `HEAD /upload`, `POST /upload`.
- Produces `UploadResponse { url: String, id: String, bytes: u64, listed: bool }`.
- Produces `upload::authenticate(state, headers) -> Result<TokenIdentity, StatusCode>` where `TokenIdentity { Env, Db(i64) }`: constant-time compare against `UPLOAD_TOKEN`; else sha256 of the presented token looked up via `db::tokens::find_active_by_hash` (Task 13 adds creation; lookup exists now).
- Produces `upload::store_upload(state, field) -> Result<Media, UploadError>` (enum `TooLarge`, `Unsupported`, `Io`, `Db`): streams to `{data_dir}/{upload_folder}/.incoming-{random}` with a running byte count, aborts and unlinks at `max_upload_bytes`, sniffs the first 8 KiB, computes id, if existing-and-available returns that row and unlinks the temp file, else renames to `{upload_folder}/{sanitised original name}` (suffix `-2`, `-3` on clash; sanitise = keep `[A-Za-z0-9._ -]`, collapse spaces, fall back to `{id}.{ext}`), inserts with `source=Upload`, `public = settings.upload_default_public`, `title = stem`, `recorded_at = recorded_at_from_name`, enqueues jobs via `jobs::enqueue_for_new_media` (Task 15; until then a no-op stub `pub async fn enqueue_for_new_media(state, &Media) {}` lives in `src/jobs/mod.rs`).
- Rate limit: `upload_limiter` keyed by token identity, 30 per 60 s.

- [ ] **Step 1: Write failing tests `tests/upload.rs`** (fixture: `common::tiny_mp4() -> Vec<u8>` returns a 1 KiB buffer beginning with a valid `ftyp` box `\0\0\0\x18ftypisom\0\0\0\0isomiso2mp41`, and `common::tiny_png()`):

```rust
#[tokio::test] async fn head_requires_bearer() { /* HEAD /upload no auth -> 401; with Bearer t -> 200 */ }
#[tokio::test] async fn post_without_token_is_401() {}
#[tokio::test] async fn upload_round_trip_returns_share_url() { /* POST multipart clip=tiny_mp4 name "Ace 2026-10-22 18-03-11.mp4" -> 200; json.url starts with base + "/w/"; json.listed == true; json.bytes == len; GET json.url -> 200 (Task 8 makes it HTML; until then assert the row exists via GET /api/library? no: assert via a second upload returning the same url) */ }
#[tokio::test] async fn duplicate_upload_returns_existing_url() {}
#[tokio::test] async fn oversize_upload_is_413_and_cleans_up() { /* MAX_UPLOAD_BYTES=600 env; 1 KiB body -> 413; data_dir/uploads has no files */ }
#[tokio::test] async fn unknown_type_is_415_and_cleans_up() { /* name "clip.exe" body "MZ..." -> 415; no file. Review Focus 1 */ }
#[tokio::test] async fn no_extension_is_415() {}
#[tokio::test] async fn upload_rate_limit_is_429() { /* 31 uploads of distinct bodies -> last is 429 */ }
#[tokio::test] async fn public_base_url_env_is_used() { /* PUBLIC_BASE_URL=https://clips.example -> url starts with it */ }
#[tokio::test] async fn image_upload_gets_i_slug() { /* png -> url contains "/i/" */ }
```

- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement `upload.rs`**; body limit via `DefaultBodyLimit::max(max_upload_bytes + 1 MiB)` on this route only so the handler, not the framework, produces 413.
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "web: upload endpoint with frozen desktop-app contract"`

---

### Task 7: Media file serving

**Files:**
- Create: `src/web/media_files.rs`, `tests/media_files.rs`
- Modify: `src/web/mod.rs`

**Interfaces:**
- Consumes `db::media::get_by_slug`, `DerivedPaths`, `db::jobs::renditions_for`.
- Produces `GET /media/{slug}/{file}` where `file` ∈ `original.{ext}` (ext must equal the row's extension), `poster.jpg`, `preview.mp4`, `thumb.jpg`, `{480|720|1080}p.mp4`. Resolves to the DB path or a `DerivedPaths` file, canonicalises, verifies the canonical path is under `data_dir` or one of `video_dirs`/`image_dirs`, and serves with `tower_http::services::ServeFile` (ranges, ETag, Last-Modified). Original gets `Content-Disposition: inline; filename="{original basename}"` with quotes escaped and a `filename*` UTF-8 variant. 404 for unavailable rows, missing derived files, or wrong ext. Never `X-Frame-Options` restriction here (media is embedded by Discord). `Cache-Control: public, max-age=3600` for derived files, `private, max-age=0` for originals of non-public media.

- [ ] **Step 1: Write failing tests** — `original_streams_with_range` (upload tiny_mp4, `Range: bytes=0-9` → 206, `Content-Range` present, 10 bytes), `range_past_end_is_416` (Review Focus 2), `wrong_extension_is_404`, `unavailable_media_is_404` (flip `available` via `db::media::set_available`), `link_only_media_is_still_served` (set public false, GET → 200), `content_disposition_has_original_name`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "web: media file serving with ranges"`

---

### Task 8: Library and clip pages (templates, CSS, JS, fonts, icons)

**Files:**
- Create: `src/web/library.rs`, `src/web/clip.rs`, `src/web/assets.rs` (rust-embed `GET /static/{*path}` with `Cache-Control: public, max-age=31536000, immutable` and a `?v={build hash}` cache-buster helper used in templates), `templates/base.html`, `templates/library.html`, `templates/clip.html`, `templates/partials/card.html`, `templates/partials/rail.html`, `templates/partials/panel_share.html`, `templates/partials/panel_details.html`, `templates/partials/panel_file.html`, `static/app.css`, `static/app.js`, `static/fonts/Oswald-{Regular,SemiBold,Bold}.ttf`, `static/fonts/Inter-{Regular,Medium,SemiBold}.ttf`, `static/icons.svg`, `static/logo.png`, `static/poster-placeholder.svg`, `tests/pages.rs`
- Modify: `src/web/mod.rs`

**Interfaces:**
- Consumes `db::media::list`, `db::views`, `ip_hash`, `client_ip`, `public_base`, `is_browser_playable`, `renditions_for`.
- Produces `GET /` (videos), `GET /api/library?kind=&q=&tag=&sort=&page=` (JSON `{ items: [LibraryItem], page, total }` where `LibraryItem { id, slug, kind, title, url, poster_url, preview_url, duration_ms, width, height, size, views, created_at, recorded_at, tags: [{name, colour}] }`), `GET /w/{slug}` (video) and `GET /i/{slug}` (image; template built fully in Task 18 but route and 404 exist now).
- Produces `clip::PlaybackSource { url: String, label: String, mime: String }` and `clip::playback_sources(media, renditions) -> Vec<PlaybackSource>` ordered best-first: original first when `is_browser_playable`, then renditions descending by height; and `clip::og_video(sources) -> Option<&PlaybackSource>` (first entry, or `None` when the list is empty).
- Produces view counting: on `GET /w|/i` record `(media_id, ip_hash(secret, client_ip))`.
- Produces template context structs `LibraryPage { title, kind, items grouped as Vec<DateSection { label: "Wed, Oct 22", items }>, counts, tags, q, sort, signed_in: bool, admin_csrf: Option<String> }` and `ClipPage { media, sources, og_video, poster_url, views, tags, share_url, signed_in, csrf, renditions }`.

Markup and style follow the spec's Frontend section and the approved mockup: title strip 36px with logo, name, search form (`GET /?q=`), sign-in link; rail 94px with Clips, Images (and Admin when signed in); page heading + count; filter chips (All / Clips / Screenshots link to `/` and `/images`; one chip per tag linking `/?tag=`); sort `<select>` that submits on change (no inline handler: `app.js` attaches it); date sections; cards with poster (`/media/{slug}/poster.jpg`, falls back to `/static/poster-placeholder.svg` via `onerror` set in JS, not inline), duration pill (`m:ss`), resolution pill (`1080p60` when fps ≥ 50 else `1080p`), tag pills, meta line, share button (`data-copy="{share_url}"`). Clip page: `<video controls preload="metadata" poster=... data-start="{t}">` with one `<source>` per PlaybackSource, quality `<select data-quality>` when more than one source, title, meta, description, right panel sections Share (copy link, download original `/media/{slug}/original.{ext}?download=1` which sets `Content-Disposition: attachment` in Task 7's handler when `download=1`, copy link at current time), Details, File. Head: `og:title`, `og:description`, `og:url`, `og:image` (poster, absolute), `og:image:width/height`, `og:type` `video.other`, and when `og_video` is `Some`: `og:video`, `og:video:secure_url` (when https), `og:video:type`, `og:video:width`, `og:video:height`, `twitter:card` `player`. Clip and image pages set the `AllowFraming` extension so `X-Frame-Options` is omitted.

`app.js` (no framework): hover-play `preview.mp4` on cards with `has_preview`, copy-to-clipboard for `[data-copy]` with a "Copied" flash, sort select submit, `?t=` seek on `loadedmetadata`, quality switching (swap `src`, keep `currentTime`), poster fallback. Responsive: rail becomes a bottom bar under 700px; grid one column under 480px.

Fonts: download Oswald (OFL) from the FTHR-Clips tag assets and Inter (OFL) from the official release; copy `licenses/Oswald-OFL-1.1.txt` into `static/fonts/OFL.txt`. Icons: copy the SVGs listed in the spec from the v2.0.0-alpha checkout at `/tmp/claude-1000/-home-kalopsia-fthr-clips-server/844e6cae-b601-4c14-86d0-423def84496c/scratchpad/fthr-clips-upstream/v2/FTHR_UI/assets/icons/` (re-clone the tag if that path is gone) into one `<symbol>` sprite; add Lucide `image`, `rss`, `tag`, `search`, `eye`, `check` symbols. Logo from `FTHR_UI/assets/fthr_logo.png`, inverted to white-on-transparent.

- [ ] **Step 1: Write failing tests `tests/pages.rs`** — `library_lists_only_public` (two uploads, one made private via `db::media::set_public` → page body contains first title, not second; `/api/library` total == 1), `library_search_and_tag_filter`, `clip_page_has_opengraph_tags` (`og:video` equals `/media/{slug}/original.mp4` absolute for a playable mp4; assert `og:image`), `clip_page_omits_og_video_when_not_playable` (set container mkv via `set_probe`), `clip_page_counts_distinct_views` (two GETs same client → views 1 in `/api/library`), `clip_page_has_no_frame_options`, `unknown_slug_is_404`, `static_assets_served` (`/static/app.css` 200, `font-src 'self'` works because no external URLs appear in CSS: assert body of app.css has no `http`).
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement templates, static assets, `library.rs`, `clip.rs`, `assets.rs`.**
- [ ] **Step 4: Run tests.** Also `cargo run -- serve` with `UPLOAD_TOKEN=t`, upload a real mp4 with curl, open `/` and `/w/{slug}` in a browser, and compare against the mockup; fix spacing and colours until it matches.
- [ ] **Step 5: Commit** — `git commit -am "web: library and clip pages in the 2.0 style"`

---

### Task 9: Admin authentication

**Files:**
- Create: `src/web/admin/mod.rs`, `src/web/admin/auth.rs`, `templates/login.html`, `tests/admin_auth.rs`
- Modify: `src/web/mod.rs`, `src/app.rs`

**Interfaces:**
- Consumes `db::sessions`, `RateLimiter`, `csrf_ok`, `request_is_https`, `argon2`.
- Produces extractor `AdminSession(Session)` (rejects with 401 JSON for `/admin/api/*`, redirect to `/admin/login?next=` for pages) and `OptionalAdmin(Option<Session>)` used by public pages for `signed_in`.
- Produces routes: `GET /admin/login` (form: password, csrf-less because no session yet; rate limited 10/min per client IP and 10/min global), `POST /admin/login` (accepts JSON `{password}` or form; verifies `ADMIN_PASSWORD` constant-time or `ADMIN_PASSWORD_HASH` via argon2; when neither set, always 401; sets cookie `fthr_admin` HttpOnly, SameSite=Lax, Path=/, Secure when `request_is_https`, Max-Age 8h; JSON callers get `{csrf}`, form callers redirect to `next` or `/admin`), `POST /admin/logout` (requires CSRF, deletes session, clears cookie), `GET /admin/api/session` → `{csrf}`.
- Produces `admin::require_csrf(session, headers, form) -> Result<(), StatusCode>` used by every state-changing admin route; `GET /admin` placeholder page "Admin" that Task 11 replaces.
- Produces `main.rs` subcommand `hash-password` (reads from stdin, prints argon2id PHC string).

- [ ] **Step 1: Write failing tests** — `login_sets_cookie_and_dashboard_is_gated` (GET `/admin` unauthenticated → 303 to login; after login → 200), `wrong_password_is_401`, `no_password_configured_always_401`, `login_rate_limited_429`, `logout_invalidates_session`, `csrf_required_on_logout` (no header → 403), `cookie_not_secure_over_http`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "admin: password login, sessions, csrf"`

---

### Task 10: Admin media and tag APIs

**Files:**
- Create: `src/web/admin/api_media.rs`, `src/web/admin/api_tags.rs`, `tests/admin_media.rs`
- Modify: `src/web/admin/mod.rs`, `src/web/clip.rs` (editable panel when signed in; Task 11 adds the JS)

**Interfaces:**
- Consumes `db::media`, `db::tags`, `DerivedPaths`, `tombstone` table.
- Produces `GET /admin/api/media?kind=&q=&tag=&sort=&page=&per_page=` (all items, including private and unavailable, with `renditions` and `jobs` summaries), `GET /admin/api/media/{id}`, `PATCH /admin/api/media/{id}` body `{title?, description?, public?, tags?: [String]}` (tags replaces the set; `attach_by_name`), `DELETE /admin/api/media/{id}?delete_file=true|false`, `POST /admin/api/media/bulk` body `{ action: "public"|"link_only"|"add_tag"|"remove_tag"|"move"|"transcode"|"remove_renditions"|"delete", ids: [String], tag?: String, folder?: String, delete_file?: bool }` → `{ ok: n, failed: [{id, error}] }`.
- Produces `api_media::delete_media(state, media, delete_file: bool) -> Result<()>`: removes `DerivedPaths`, renditions, tags, views, jobs, row. Deletes the source file when `media.source == Upload` and the path is under `{data_dir}/{upload_folder}`, or when `delete_file` is true and the canonical path is under a configured scan dir. Otherwise inserts `tombstone(id, path)` so the scanner ignores it.
- Produces `api_media::move_media(state, media, folder) -> Result<()>`: `folder` must be a relative path with no `..`; target root is `{data_dir}/{upload_folder}` for uploads or the scan root the file lives under; renames the file, updates `path` and `folder`.
- Produces tags: `GET /admin/api/tags`, `POST` `{name, colour}`, `PATCH /admin/api/tags/{id}` `{name?, colour?}`, `DELETE /admin/api/tags/{id}`. Colour must match `^#[0-9a-fA-F]{6}$`.

- [ ] **Step 1: Write failing tests** — `patch_updates_title_and_tags`, `unlist_hides_from_public_library_then_relist_shows`, `delete_upload_removes_file_and_404s` (file under uploads gone; `/w/{slug}` → 404), `delete_scanned_file_keeps_file_without_flag` (insert a row pointing at a temp file outside data_dir via `db::media::insert`; DELETE without flag → file still exists, tombstone row present; with `delete_file=true` → gone), `bulk_add_tag_and_link_only`, `tag_colour_validated_400`, `admin_api_requires_session_401`, `admin_api_requires_csrf_403`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "admin: media and tag apis"`

---

### Task 11: Admin pages: Files and Tags

**Files:**
- Create: `src/web/admin/pages.rs`, `templates/admin/base.html` (tab bar), `templates/admin/files.html`, `templates/admin/tags.html`, `static/admin.js`, `tests/admin_pages.rs`
- Modify: `src/web/admin/mod.rs`, `templates/clip.html` (Visibility, Details editable, Delete button when signed in; `data-csrf` on `<body>`)

**Interfaces:**
- Produces `GET /admin` → redirect `/admin/files`; `GET /admin/files` (stats row placeholders filled by `/admin/api/stats` in Task 15; table rendered server-side from `db::media::list` with `public_only=false`, per_page 50, pagination links; checkboxes `data-id`; bulk bar hidden until a selection), `GET /admin/tags`.
- `admin.js`: selection state, bulk bar actions calling `/admin/api/media/bulk` with `X-CSRF-Token` from `body[data-csrf]`, delete confirm dialog with the "also delete the file from disk" checkbox shown only for rows with `data-source="scan"`, row-level visibility toggle, tag create/rename/recolour/delete, clip page inline editing (Details save → PATCH, Visibility segmented control → PATCH, Delete → DELETE then redirect `/`). After each action, reload the page (no client-side rendering of user strings).

Layout per the approved mockup: tab bar `Files · Tags · Library · Transcoding · Sharing · Integrations · Tokens` (later tabs link to pages built in Tasks 12 and 13; until then they 404), stats row, bulk bar, table columns: checkbox, poster + title, visibility pill, tags, views, size, status pill, added.

- [ ] **Step 1: Write failing tests** — `admin_files_page_lists_private_items`, `admin_files_page_is_gated`, `clip_page_shows_editor_when_signed_in` (body contains `data-csrf`, the Visibility segmented control, and a Delete button; unauthenticated body does not).
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.** Verify by hand in the browser against the mockup's Admin tab.
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "admin: files and tags pages"`

---

### Task 12: Settings API and pages (Library, Transcoding, Sharing, Integrations)

**Files:**
- Create: `src/web/admin/api_settings.rs`, `templates/admin/library.html`, `templates/admin/transcoding.html`, `templates/admin/sharing.html`, `templates/admin/integrations.html`, `tests/admin_settings.rs`
- Modify: `src/web/admin/pages.rs`, `src/web/library.rs` and `src/web/feed.rs` (later) honour `library_requires_login`

**Interfaces:**
- Consumes `SettingsStore`.
- Produces `GET /admin/api/settings` → `Settings` JSON plus `effective_encoder` (filled in Task 14; `null` until then), `PUT /admin/api/settings` body = partial `Settings` → updated JSON. Each page is a plain HTML `<form method=post>` posting to `POST /admin/{tab}` which converts form fields to the patch and redirects back with `?saved=1`.
- Produces gating: when `library_requires_login` is true and no admin session, `GET /`, `/images`, `/api/library`, `/feed.rss` redirect (pages) or 401 (API/feed) to login; `/w`, `/i`, `/media` untouched.

- [ ] **Step 1: Write failing tests** — `put_settings_persists_across_restart` (spawn, PUT `library_requires_login: true`, spawn again on the same data dir → `/` redirects), `library_login_gate_leaves_share_links_open` (`/w/{slug}` 200, `/` 303), `sharing_form_post_updates_site_title` (page `<title>` changes).
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "admin: settings api and pages"`

---

### Task 13: Upload tokens

**Files:**
- Create: `src/web/admin/api_tokens.rs`, `templates/admin/tokens.html`, `tests/tokens.rs`
- Modify: `src/web/admin/pages.rs`, `src/web/upload.rs` (touch `last_used_at`)

**Interfaces:**
- Produces `GET /admin/api/tokens` (never returns hashes), `POST /admin/api/tokens` `{name}` → `{id, name, token}` (token = `fthr_` + 32 base64url chars, shown once; stored sha256; `prefix` = first 12 chars), `DELETE /admin/api/tokens/{id}` (revoke). Page lists name, prefix, created, last used, revoke button, create form that shows the new token in a copyable box once.

- [ ] **Step 1: Write failing tests** — `created_token_uploads_and_env_token_still_works`, `revoked_token_is_401`, `last_used_updates_after_upload`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "admin: upload tokens"`

---

### Task 14: ffmpeg integration: locate, probe, progress, encoder detection, `check`

**Files:**
- Create: `src/ffmpeg/mod.rs`, `src/ffmpeg/probe.rs`, `src/ffmpeg/encoders.rs`, `tests/ffmpeg.rs`, `tests/fixtures/make_fixture.rs` (helper that generates `tests/fixtures/tiny.mp4` with `ffmpeg -f lavfi -i testsrc=duration=2:size=320x240:rate=30 -f lavfi -i sine=duration=2 -c:v libx264 -c:a aac -pix_fmt yuv420p` on first use into the temp dir)
- Modify: `src/main.rs` (`check` subcommand), `src/app.rs` (AppState gains `ffmpeg: Arc<Ffmpeg>`)

**Interfaces:**
- Produces `Ffmpeg::locate(config) -> Ffmpeg { ffmpeg: Option<PathBuf>, ffprobe: Option<PathBuf>, version: Option<String> }` (explicit paths, else `which` on PATH; never errors), `available() -> bool`.
- Produces `probe::ProbeInfo { container: String, duration_ms: Option<i64>, width, height, vcodec: Option<String>, acodec: Option<String>, fps: Option<f64>, bitrate: Option<i64>, creation_time: Option<DateTime<Utc>>, raw: serde_json::Value }` and `Ffmpeg::probe(&self, path) -> Result<ProbeInfo>` running `ffprobe -v error -print_format json -show_format -show_streams`.
- Produces `Ffmpeg::run(&self, args: &[OsString], timeout: Duration, on_progress: impl FnMut(f64)) -> Result<()>`: adds `-nostdin -hide_banner -y -progress pipe:1`, parses `out_time_us` against a supplied total to call `on_progress(0.0..=1.0)`, kills on timeout, returns stderr tail in the error.
- Produces `encoders::Encoder { X264, Nvenc, Vaapi, Qsv, VideoToolbox }` with `video_args(&self, height: u32, fps: f64) -> Vec<OsString>` (x264: `-c:v libx264 -preset fast -crf 23`; nvenc: `-c:v h264_nvenc -preset p4 -cq 23`; vaapi: `-vaapi_device /dev/dri/renderD128 -vf format=nv12,hwupload,scale_vaapi=-2:{h} -c:v h264_vaapi -qp 23`; qsv: `-c:v h264_qsv -global_quality 23`; videotoolbox: `-c:v h264_videotoolbox -q:v 60`; all non-vaapi add `-vf scale=-2:{h}`; all add `-maxrate` per height: 480→2.5M, 720→5M (8M when fps>50), 1080→8M (12M when fps>50), `-bufsize` 2× maxrate, `-c:a aac -b:a 128k -movflags +faststart -pix_fmt yuv420p` except vaapi) and `encoders::detect(ffmpeg, choice: EncoderChoice) -> Option<Encoder>`: for `Auto` try nvenc, qsv, vaapi, videotoolbox, x264 in that order; each candidate is a 1-second `-f lavfi -i testsrc=duration=1:size=256x144:rate=30` encode to `-f null -` with a 20 s timeout; first success wins. Explicit choice tests only that encoder.
- Produces `check` subcommand output: data dir, db ok, ffmpeg path + version or "not found", encoder result, scan dirs with existence, bind address.

- [ ] **Step 1: Write failing tests** (each returns early with `eprintln!("skipped: ffmpeg not found")` when `Ffmpeg::locate` finds nothing) — `probe_reports_duration_and_codecs` on the fixture (duration ≈ 2000 ms, vcodec `h264`, container `mov,mp4,m4v,3gp,3g2,mj2` normalised to `mp4` by `ProbeInfo::container` normalisation: take the file's extension when the format_name list contains it), `run_reports_progress` (a 2 s re-encode to `-f null -` gets at least one progress callback with value in 0..=1), `detect_x264_succeeds`, `locate_without_binaries_is_not_an_error` (set `FFMPEG_PATH=/nonexistent` → `available() == false`).
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.** Also run `cargo run -- check`.
- [ ] **Step 5: Commit** — `git commit -am "ffmpeg: locate, probe, progress, encoder detection"`

---

### Task 15: Job queue, workers, poster/preview/thumb jobs, admin stats and events

**Files:**
- Create: `src/jobs/mod.rs` (replace stub), `src/jobs/worker.rs`, `src/jobs/poster.rs`, `src/jobs/thumb.rs`, `src/web/admin/api_jobs.rs`, `tests/jobs.rs`
- Modify: `src/app.rs` (start workers in `serve`; AppState gains `events: EventBus`), `templates/admin/files.html` (queue section + stats row), `static/admin.js` (SSE subscribe, fall back to polling `/admin/api/jobs` every 5 s)

**Interfaces:**
- Produces `JobKind` strings: `probe`, `poster`, `preview`, `thumb`, `transcode:{h}`.
- Produces `jobs::enqueue_for_new_media(state, &Media)`: video → `probe`, `poster`, `preview`; image → `thumb`. Transcode enqueueing is added in Task 16 and runs after `probe` completes.
- Produces `EventBus` (tokio `broadcast::Sender<JobEvent>`, capacity 256) with `JobEvent { job_id, media_id, kind, state, progress, error }`.
- Produces `worker::spawn(state, name, kinds: Vec<String>, concurrency)`; `serve` spawns one "light" pool (kinds probe/poster/preview/thumb, concurrency 2) and one "heavy" pool (transcode:*, concurrency `settings.transcode_workers`). Loop: `claim_next`, run, `finish`/`fail` (fail after 3 attempts stays failed; earlier attempts re-queue with `attempts+1`), broadcast events. When `ffmpeg.available()` is false, ffmpeg-dependent kinds are not claimed (they stay `queued`). Cancellation: a `CancellationToken` per running job kept in `AppState.running: Mutex<HashMap<i64, CancellationToken>>`; `cancel` sets state and triggers the token; `Ffmpeg::run` takes the token and kills the child.
- Produces jobs: `probe` → `set_probe` + `recorded_at` fallback from `creation_time` when null + `processing_state = ready` if no poster needed; `poster` → `ffmpeg -ss {dur*0.1} -i src -frames:v 1 -vf scale=-2:720 poster.jpg` retrying at `-ss 0`, then `set_flags(has_poster)`; `preview` → `-ss {dur/2 - 2, min 0} -t 4 -vf scale=-2:360 -an -c:v libx264 -preset veryfast -crf 30 -movflags +faststart preview.mp4` (skipped silently when the x264 encoder test fails); `thumb` → `image` crate open, resize longest edge to 720, save JPEG q85 to `thumb.jpg`, also store width/height into the media row.
- Produces `GET /admin/api/jobs` (active + last 50 finished), `POST /admin/api/jobs/{id}/cancel`, `POST /admin/api/jobs/{id}/retry`, `GET /admin/api/events` (SSE of `JobEvent` + a `scan` event type from Task 16), `GET /admin/api/stats` → `{ media: {videos, images}, disk: {used, total, path}, last_scan: {at, found} | null, ffmpeg: {available, version}, encoder: String | null, workers: n }` (disk via `nix`-free approach: `fs2`-style `available_space` from the `sysinfo` crate or `statvfs` through `libc` on unix and `GetDiskFreeSpaceExW` on windows; pick `sysinfo` with `disk` feature for portability).

- [ ] **Step 1: Write failing tests `tests/jobs.rs`** — `poster_and_probe_run_after_upload` (skip without ffmpeg; upload fixture mp4, poll `/admin/api/media/{id}` until `has_poster` within 30 s, assert `duration_ms` set and `/media/{slug}/poster.jpg` 200), `thumb_runs_without_ffmpeg` (`FFMPEG_PATH=/nonexistent`; upload png; `thumb.jpg` 200 within 10 s), `jobs_stay_queued_without_ffmpeg_and_pages_render` (Review Focus 5: `FFMPEG_PATH=/nonexistent`; upload mp4; `/` 200 and `/w/{slug}` 200 contain the placeholder poster path; `/admin/api/jobs` shows `poster` queued), `cancel_and_retry_change_state`, `stats_endpoint_shape`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "jobs: queue, workers, poster/preview/thumb, admin stats and events"`

---

### Task 16: Scanner

**Files:**
- Create: `src/media/scan.rs`, `tests/scan.rs`
- Modify: `src/app.rs` (start scheduler + watcher in `serve`), `src/main.rs` (`scan` subcommand runs one pass and exits), `src/web/admin/api_jobs.rs` (`POST /admin/api/scan`)

**Interfaces:**
- Consumes `media_id`, `slug_candidates`, `kind_for`, `recorded_at_from_name`, `db::media`, `tombstone`, `enqueue_for_new_media`.
- Produces `Scanner::new(state)`, `Scanner::run_once(&self) -> Result<ScanReport { found: u32, relinked: u32, missing: u32, skipped_growing: u32 }>`; roots = `{data_dir}/{upload_folder}` (always) + `video_dirs` + `image_dirs`; recursive via `walkdir`, following symlinks inside roots only; extension allowlists from `Settings`; skip hidden files, files whose size changed between two stats 2 s apart or whose mtime is within the last 10 s, and paths in `tombstone`. For each candidate: `get_by_path` → known, mark available if it was not; else compute id; `get(id)` exists: if that row's path is missing on disk → `relink`, else skip duplicate; else insert with `source=Scan`, `public=true` for scan roots? No: scanned files default to **link-only** (`public=false`) so a user's whole recordings folder is never published by accident; uploads use `upload_default_public`. `folder` = path relative to its root, parent dir. After the walk, every row whose path was not seen and still has `source=Scan` is `set_available(false)`; upload rows likewise. Report stored in `AppState.last_scan: RwLock<Option<(DateTime<Utc>, ScanReport)>>` and broadcast as `scan` event.
- Produces scheduler: `tokio::spawn` loop every `scan_interval_minutes` (re-read from settings each tick; 0 disables) and a `notify` recommended watcher on all roots with a 3 s debounce that triggers `run_once`. Startup runs one pass. A `Mutex<()>` ensures only one scan runs at a time; a request during a scan sets a "rerun" flag.

- [ ] **Step 1: Write failing tests `tests/scan.rs`** — `scan_picks_up_files_in_video_dir` (env `VIDEO_DIRS={tmp}`; write tiny_mp4 as `2026-10-21 10-00-00.mp4`; `POST /admin/api/scan`; `/admin/api/media` shows it with `public=false`, `source="scan"`, `recorded_at` set), `growing_file_is_skipped_then_found` (Review Focus 3: write file, keep appending during the scan via a task, `skipped_growing==1`; stop appending, scan again → found), `moved_file_is_relinked_not_duplicated` (scan; move file to a subfolder; scan → same id, new path, 1 row), `missing_file_marked_unavailable_and_hidden` (delete file; scan; `available=false`; `/` does not list it; `/w/{slug}` 404), `tombstoned_file_is_ignored`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "scan: folder scanning, relink, unavailable, watcher"`

---

### Task 17: Transcoding

**Files:**
- Create: `src/jobs/transcode.rs`, `tests/transcode.rs`
- Modify: `src/jobs/mod.rs` (after `probe`: decide enqueue per mode), `src/web/admin/api_media.rs` (`POST /admin/api/media/{id}/transcode` body `{heights:[u32]}`, `DELETE /admin/api/media/{id}/renditions/{h}`, bulk `transcode` / `remove_renditions`), `templates/clip.html` + `templates/partials/panel_quality.html` (Quality section: per rung state/progress, queue/remove buttons for admin), `static/admin.js`, `templates/admin/transcoding.html` (shows `effective_encoder`)

**Interfaces:**
- Produces `transcode::plan_for(settings, probe: &ProbeInfo, extension) -> Vec<u32>`: `Off` → `[]`; `Compat` → `[]` when `is_browser_playable`, else `[max rung ≤ source height, else smallest rung]`; `On` → every ladder rung ≤ source height (never upscale; source height < 480 → `[]` unless not playable, then `[480]` scaled to source height via `-2:min(480,h)`).
- Produces job `transcode:{h}`: `rendition_upsert(state=running)`, `ffmpeg.run([-i src, encoder.video_args(h,fps)..., tmp])` with timeout `transcode_timeout_minutes`, progress from `out_time_us / duration`, rename tmp → `DerivedPaths::rendition(h)`, `rendition_ready`. Failure → rendition `failed` with error. Removing a rendition deletes the file and row.
- Produces `AppState.encoder: RwLock<Option<Encoder>>` detected at startup (spawned, not blocking bind) and re-detected when the encoder setting changes.
- Clip page's `playback_sources` now includes ready renditions; `og:video` follows the spec rule (original if playable else highest ready rendition).

- [ ] **Step 1: Write failing tests** (skip without ffmpeg) — `compat_mode_skips_playable_mp4` (upload fixture; wait for probe; `/admin/api/media/{id}` shows no renditions and no transcode jobs), `compat_mode_transcodes_mkv` (remux fixture to `.mkv` with ffmpeg `-c copy` in the test; upload; wait ≤ 60 s for `720p.mp4` 200; clip page `og:video` ends with `/720p.mp4`), `manual_transcode_and_remove` (POST transcode `[480]` → wait → `/media/{slug}/480p.mp4` 200; DELETE → 404), `plan_for_rules` (unit: on-mode with 1080 source and ladder [480,720,1080] → all three; 720 source → [480,720]).
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "transcode: compat/on modes, renditions, quality picker"`

---

### Task 18: Images page and image view

**Files:**
- Create: `templates/image.html`, `templates/images.html` (or reuse `library.html` with `kind=Image`), `tests/images.rs`
- Modify: `src/web/library.rs` (`GET /images`), `src/web/clip.rs` (`GET /i/{slug}` full page), `static/app.js` (lightbox: click thumb → full-screen overlay showing `original`, Esc closes)

**Interfaces:**
- `/images` is the library page with `kind=Image`, heading "Screenshots", cards show `thumb.jpg`, no duration pill, format pill (`PNG`). `/i/{slug}`: image centred at max 1200px wide, same right panel (Share, Details, File; editable when signed in). Head: `og:image` = original, `og:image:width/height`, `og:type` `website`, `twitter:card` `summary_large_image`.

- [ ] **Step 1: Write failing tests** — `images_page_lists_only_images_that_are_public`, `image_page_has_og_image_of_original`, `image_page_counts_views`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "web: images library and image page"`

---

### Task 19: RSS feed

**Files:**
- Create: `src/web/feed.rs`, `tests/feed.rs`
- Modify: `src/web/mod.rs`, `templates/base.html` (`<link rel="alternate" type="application/rss+xml">`), rail RSS link

**Interfaces:**
- Produces `GET /feed.rss`: `application/rss+xml; charset=utf-8`; channel title = `site_title`, link = public base, description "Public clips"; 50 newest public items; each item: title, link (`/w|/i` absolute), guid (same, permaLink), pubDate (RFC 2822 from `recorded_at` else `created_at`), description (escaped text), `<enclosure url=... length=size type=mime>` pointing at the best playback source (original for images), and `<media:thumbnail url=poster|thumb>` with the `media` namespace declared. Honour `library_requires_login` (401).

- [ ] **Step 1: Write failing tests** — `feed_lists_public_only_with_enclosure` (parse with the `rss` crate: `Channel::read_from`; items.len()==1; enclosure url ends with `/original.mp4`), `feed_is_gated_when_library_requires_login`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "web: rss feed"`

---

### Task 20: Webhooks

**Files:**
- Create: `src/webhooks.rs`, `tests/webhooks.rs`
- Modify: `src/jobs/poster.rs` and `src/jobs/thumb.rs` (call `webhooks::maybe_announce` after success), `src/web/admin/api_media.rs` (call after `set_public(true)`)

**Interfaces:**
- Produces `webhooks::maybe_announce(state, media_id)`: if the media is public and has poster/thumb and no `webhook_sent` row, insert the row then POST: Discord `{ "embeds": [{ "title", "url": share_url, "description" (≤ 200 chars), "image": {"url": poster_url}, "color": 2155424 }] }`; generic: `LibraryItem` JSON. Both with a 10 s timeout, errors logged, never retried, never blocking the job.

- [ ] **Step 1: Write failing tests** — spin a tiny axum server in the test that records request bodies; set `WEBHOOK_URL` and `DISCORD_WEBHOOK_URL` to it; upload a png (no ffmpeg needed); assert one Discord body with `embeds[0].url` containing `/i/` and one generic body with `slug`; second `PATCH public=false` then `true` → still one send.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "webhooks: discord and generic on first public"`

---

### Task 21: Legacy import and `/files/{old_id}` redirect

**Files:**
- Create: `src/legacy.rs`, `src/web/legacy.rs`, `tests/legacy.rs`
- Modify: `src/main.rs` (`import-legacy` subcommand), `src/app.rs` (auto-run on startup when `data_dir` has `*.json` sidecars and no `library.db` existed before open)

**Interfaces:**
- Produces `legacy::import(state) -> Result<ImportReport { imported: u32, skipped: u32 }>`: for each `{data_dir}/{id}.json` with sibling file `{data_dir}/{id}` where `id` matches `^[A-Za-z0-9_-]{20,32}$`: read `{name, content_type, listed}`; determine kind from `name`'s extension + sniff (skip and log when unknown); move the file to `{upload_folder}/{sanitised name}` (clash suffix); insert media with `source=Upload`, `public=listed`, `title=stem`, `created_at` = file mtime; insert `legacy_id(old_id, media_id)`; delete the sidecar; enqueue jobs. Idempotent: an `old_id` already in `legacy_id` is skipped.
- Produces `GET /files/{old_id}` → 301 to `/w|/i/{slug}`; 404 when unknown.

- [ ] **Step 1: Write failing tests** — `import_moves_files_and_redirects` (write `abc...24chars` + sidecar `{"name":"Ace.mp4","content_type":"video/mp4","listed":true}` into a fresh data dir before spawn; after spawn `/files/{old}` → 301 to `/w/...`; `uploads/Ace.mp4` exists; sidecar gone; media public), `import_is_idempotent`.
- [ ] **Step 2: Run to verify failure.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run tests.**
- [ ] **Step 5: Commit** — `git commit -am "legacy: import old uploads and redirect old links"`

---

### Task 22: Deploy, CI, README

**Files:**
- Create: `Dockerfile`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `.dockerignore`, `config.example.toml`, `.env.example`
- Modify: `docker-compose.yml` (comments only: optional `/videos`, `/images` mounts, `mem_limit: 2g` suggestion), `README.md`

**Interfaces:**
- `Dockerfile`: stage 1 `rust:1-bookworm` builds `--release` for the native arch (buildx provides amd64 and arm64 via `--platform`); stage 2 `debian:bookworm-slim` with `apt-get install -y --no-install-recommends ffmpeg ca-certificates`, user `10001`, `/data` volume, `ENV DATA_DIR=/data BIND_HOST=0.0.0.0 PORT=8080`, `EXPOSE 8080`, `HEALTHCHECK` on `/healthz`, `CMD ["fthr-clips-server", "serve"]`.
- `ci.yml`: on push/PR: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` on `ubuntu-latest` (with `apt-get install ffmpeg`), `macos-latest`, `windows-latest` (without ffmpeg; the skips exercise the graceful path).
- `release.yml`: on tag `v*`: matrix builds with `cross` for the two musl targets and native runners for macOS (both arches via `-target`) and Windows; upload tarballs/zip to the GitHub release; buildx multi-arch image pushed to `ghcr.io/<owner>/fthr-clips-server:{tag,latest}`.
- README sections: what it is (one line: built by an FTHR maintainer, not an official FTHR Community release), quick start (binary, Docker, compose), desktop app setup ("Your server": URL `https://host/upload`, Authorization `Bearer <token>`), configuration table (every env var with default), transcoding modes, scan folders, reverse proxy note (`TRUSTED_PROXIES`), migrating from the Python version, development (`cargo test`, fixtures need ffmpeg), license GPL-3.0-only.

- [ ] **Step 1: Write `Dockerfile`, `.dockerignore`, compose comments.**
- [ ] **Step 2: Verify** — `docker build -t fthr-clips-server .` then `docker run --rm -e UPLOAD_TOKEN=t -e ADMIN_PASSWORD=p -p 18080:8080 fthr-clips-server` and `curl -I -H 'Authorization: Bearer t' localhost:18080/upload` → 200; `docker compose config` validates.
- [ ] **Step 3: Write workflows and README, `config.example.toml`, `.env.example`.**
- [ ] **Step 4: Verify** — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` locally; `cross build --release --target aarch64-unknown-linux-musl` if `cross` is installed (otherwise note it in the handoff).
- [ ] **Step 5: Commit** — `git commit -am "deploy: dockerfile, ci, release builds, readme"`

---

## Self-review notes

- Spec coverage: every spec section maps to a task (contract → 6, serving → 7, pages → 8/18, admin → 9–13, pipeline → 14–17, feed → 19, webhooks → 20, migration → 21, deploy/tests → 22 and per-task tests). Light theme and mobile bottom bar are in Task 8. `check` subcommand in 14, `scan` in 16, `hash-password` in 9, `import-legacy` in 21.
- Deviation from spec, recorded here: image thumbnails are `thumb.jpg`, not `thumb.webp`, because the `image` crate's WebP encoder is lossless-only and large. Scanned files default to link-only rather than the upload default; the spec's "public = the configured default for its source" is satisfied by treating the scan source default as link-only.
- Type consistency checked: `Media`, `ProbeInfo`, `Settings`, `JobEvent`, `PlaybackSource`, `DerivedPaths` names are used identically across tasks.
- Review Focus items are pinned to Tasks 6, 7, 16, 5, 15 respectively.
