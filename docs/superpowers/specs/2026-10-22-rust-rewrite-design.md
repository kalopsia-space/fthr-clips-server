# FTHR Clips Server: Rust rewrite design

Date: 2026-10-22
Status: ready for owner approval

## Intent

Throw away the current Python service and rebuild it from scratch in Rust as a
self-hosted clip library in the spirit of Fireshare, with the look and feel of
the FTHR Clips 2.0 desktop app. The owner's reasons, in their words: Rust is
better than Python, the feature set should be similar to Fireshare, the UI
should look like FTHR Clips 2.0, and it must work on as many setups as
possible because this is a public side project by a single FTHR maintainer.

The existing service is small (one Python file) but it has one contract that
must survive: the FTHR Clips desktop app's "Your server" upload provider
posts to it. Breaking that breaks every existing user.

## Goals

1. A single Rust binary that serves a public clip and image library, a clip
   page with a share link that embeds in Discord, and an admin area.
2. Feature parity with Fireshare's core: folder scanning, hash-based IDs,
   ffmpeg posters and metadata, public vs link-only, tags, search, sort,
   views, bulk admin management, webhooks.
3. Plus the three extras the owner asked for: transcoding to a quality
   ladder, an image library alongside videos, and an RSS feed.
4. Keep the desktop app upload contract byte-for-byte compatible.
5. Run anywhere: Linux, macOS and Windows on x86_64 and aarch64 (Oracle free
   tier ARM included), as a bare binary or a Docker image, with or without
   a GPU, and degrade gracefully when ffmpeg is missing.
6. Carry forward the security posture of the current service: token auth
   for uploads, password admin with CSRF, rate limits, hardened headers,
   path-traversal and symlink defences on file serving.

## Non-goals

- Multi-user accounts, roles, invites, MFA, public user profiles.
- Password-protected individual videos.
- SteamGridDB game metadata or any game detection.
- HDR tonemapping, cropping, editing of any kind. Editing lives in the
  desktop app.
- Replicating Fireshare's API surface. We take its features, not its routes.
- A JavaScript build toolchain. The frontend is server-rendered HTML with a
  small amount of hand-written JS and CSS, embedded in the binary.

## Anti-goals

Things that would make this a failure even if everything technically works:

- The desktop app's upload stops working, or old share links die silently.
- It only runs well on one kind of machine. A hard dependency on NVENC,
  glibc versions, or x86 is a failure.
- A broken or missing ffmpeg takes the whole site down. Serving must never
  depend on processing.
- The UI reads as a generic admin template instead of the 2.0 app.

## Constraints

- Language: Rust, stable toolchain, 2021 edition.
- Storage: SQLite, embedded, single file in the data directory. No external
  database ever.
- External tools: ffmpeg and ffprobe on PATH, optional at runtime.
- The desktop upload contract (see below) is frozen.
- Existing env var names keep working: `UPLOAD_TOKEN`, `ADMIN_PASSWORD`,
  `STORAGE_DIR`, `PUBLIC_BASE_URL`, `MAX_UPLOAD_BYTES`, `BIND_HOST`, `PORT`.
- The existing `docker-compose.yml` must still start the new image with no
  edits other than optionally adding the videos and images volumes.
- License: GPL-3.0-only, the owner's decision, matching FTHR-Clips. A
  `LICENSE` file is added. This also means the 2.0 app's icon set can be
  reused as-is.

## What it is, from the user's side

### Public visitors

- **Library** at `/`. Shell is the 2.0 app's: a 36px title strip with the
  FTHR logo and name, a search box, and a sign-in button; a 94px left rail
  with icon-over-label buttons for Clips and Images; content area with a
  page heading and item count, filter chips (All, Clips, Screenshots, plus
  one chip per tag), and a sort select (newest, oldest, most viewed, title,
  longest). Items are grouped under date section headers ("Wed, Oct 22")
  like the desktop grid. Cards are 16:9 poster plus a body: title, meta line
  (size, relative time, views), tag pills, and an accent share button that
  copies the link. Hover lifts the card, shows a play icon, and plays the
  boomerang preview if one exists. Only public items appear here.
- **Images** at `/images`: same layout, image thumbnails, lightbox on click.
- **Clip page** at `/w/{slug}` (videos) and `/i/{slug}` (images). Player on
  the left with a quality picker listing whatever renditions exist, title,
  recorded date, resolution, size, views, description. Right panel of
  collapsible sections: Share (copy link, download original, copy link at
  current time), Details and File (read-only for visitors). The page's
  `<head>` carries OpenGraph and Twitter card tags (`og:video`, `og:image`,
  width, height, title, description, `og:type=video.other`) so Discord
  embeds the video inline. Link-only items are reachable here but never
  listed, searched, fed or counted on the library page.
- `?t=42` on a clip page seeks the player to 42 seconds.
- **RSS** at `/feed.rss`: the public library, newest first, one `<item>` per
  public clip or image with title, description, link, pubDate, and an
  `<enclosure>` for the original file plus a `<media:thumbnail>`.
- Search is server-side (title, description, tag names) and returns the
  library page filtered.
- Views: one view per distinct client IP per item, recorded on clip page
  load. The client IP comes from the socket unless `TRUSTED_PROXIES` lists
  the peer, in which case the rightmost untrusted `X-Forwarded-For` hop.

### Admin

Single admin, password login at `/admin/login`. When signed in, the rail gains
an Admin button and the clip page's right panel becomes editable: Visibility
(public / link-only), Details (title, description, tags), Quality (renditions
with progress, buttons to queue or remove transcodes), Delete.

Admin area at `/admin` uses the 2.0 settings page pattern: a top tab bar and a
sliding content area. Tabs:

- **Files**: stats row (media count, disk usage of the data volume, last scan
  and how many it found, active encoder and worker count); bulk manager table
  with checkboxes, poster thumbnail, title, visibility, tags, views, size,
  processing status, added; bulk actions: make public, link only, add tag,
  remove tag, move to folder, queue transcode, remove transcodes, delete; a
  "Scan now" button; the processing queue with progress, cancel and retry.
  Delete always removes the library row and the derived files. The source
  file is deleted from disk when it lives under the server-owned `uploads/`
  folder. For files found in a scanned folder the confirm dialog has an
  unticked "also delete the file from disk" box; left unticked, the file
  stays and is ignored by future scans until the row is purged (hash kept
  in a tombstone). Move offers only folders under the configured roots and
  `uploads/`; it never creates paths outside them.
- **Tags**: create, rename, recolour, delete tags.
- **Library**: scan folder paths (videos, images), scan interval, file
  extensions, whether uploads from the desktop app default to public or
  link-only, which folder uploads land in.
- **Transcoding**: mode (off, compat, on; default compat, see pipeline),
  encoder (auto, x264, nvenc, vaapi, qsv, videotoolbox), quality ladder
  (480p, 720p, 1080p toggles; default 720p only), workers (default 1),
  timeout.
- **Sharing**: public base URL, site title, library default sort, and
  "library requires sign-in". That last setting hides the listing pages,
  search, the feed and the library API behind admin login. Clip pages and
  media files stay reachable by slug so existing share links and Discord
  embeds keep working; the server becomes "everything is link-only".
- **Integrations**: Discord webhook URL, generic JSON webhook URL, fire on
  new public media.
- **Tokens**: upload tokens for the desktop app: create (shown once),
  name, prefix, last used, revoke.

Settings edited in the admin UI are stored in the database and override
environment defaults; the env vars remain the bootstrap and the Docker-only
path.

## The upload contract (frozen)

This is what the FTHR Clips desktop app "Your server" provider does, and what
the current service implements. The rewrite must match it exactly.

- `POST {upload_url}` as `multipart/form-data`, file field `clip`, header
  `Authorization: Bearer <token>` (the app sends the header value verbatim).
- Success: 2xx with JSON `{"url": "<share url>", "id": "...", "bytes": N,
  "listed": bool}`. Verified against the app's `FTHR_Uploader/upload_runtime.py`
  at tag v2.0.0-alpha: the app reads only `url` (falling back to `raw_url`),
  and accepts a bare URL body too. The other fields are informational and
  may be properly typed (today's service emits them as strings; nothing
  depends on that).
- What the app sends: files from its own library, which are `.mp4`, `.mkv`
  (lossless export) or `.avi` for clips and `.png`, `.jpg`, `.jpeg` for
  screenshots, with `Content-Type` guessed from the extension. Accept the
  full scan allowlist below plus `.avi`. Kind is decided by extension, then
  confirmed by sniffing the first bytes; a mismatch or unknown type is 415.
  Today's service has no 415 and accepts anything, so 415 is new but only
  reachable with files the app never produces.
- Duplicate upload (hash already known and the file still exists): discard
  the new bytes and answer 200 with the existing item's URL. Idempotent
  retries from the app then just work.
- `HEAD {upload_url}` is the app's connection test. 401, 404 and 405 count
  as reachable; the current service answers 200 with the bearer and 401
  without. Keep that.
- Errors: 401 bad token, 413 too large (partial file removed), 415
  unsupported type, 429 rate limited.
- Path: `/upload`. Keep it.

The share URL returned is the clip page `/w/{slug}` (or `/i/{slug}` for an
image), absolute, built from the configured public base URL, falling back to
the request's `Host` and forwarded proto when none is configured.

Tokens: the `UPLOAD_TOKEN` env var is accepted as a token forever. Additional
tokens are created in the admin Tokens tab, stored as SHA-256 hashes with a
display prefix. The response `listed` reflects whether the upload became
public (per the Library setting).

## Architecture

One binary, `fthr-clips-server`. Subcommands: `serve` (default), `scan`,
`import-legacy`, `hash-password`, `check` (prints detected ffmpeg, encoders,
config).

### Crates (builder may substitute equivalents)

- `axum` + `tokio` + `tower-http` (compression, request ids, static serving
  with range support, timeouts, body limits).
- `sqlx` with the bundled SQLite driver and compile-time-checked queries, or
  `rusqlite` + a thin query layer. Migrations embedded and run at startup.
- `askama` templates, `rust-embed` for static assets (fonts, CSS, JS, logo).
- `xxhash-rust` (xxh3-128) for content IDs.
- `argon2` for the admin password hash; `subtle` for constant-time compares.
- `notify` for filesystem events plus a periodic full scan.
- `tokio::process` for ffmpeg and ffprobe; `image` crate for image
  thumbnails so images work without ffmpeg.
- `serde`, `toml` for config; `clap` for CLI; `tracing` for logs;
  `reqwest` (rustls) for webhooks.
- `rss` or hand-written XML for the feed.

Everything builds for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
`x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
No OpenSSL, no C dependencies beyond bundled SQLite.

### Directories

- `DATA_DIR` (alias `STORAGE_DIR`, default `/data` in Docker, `./data`
  otherwise): `library.db`, `uploads/` (where desktop uploads land, also a
  scanned folder), `derived/{id}/` (poster.jpg, preview.mp4, 480p.mp4,
  720p.mp4, 1080p.mp4, thumb.webp for images), `config.toml` (optional).
- `VIDEO_DIRS` and `IMAGE_DIRS`: comma-separated extra folders to scan,
  read-only from the server's point of view except for delete and move,
  which only happen on explicit admin action.

### Data model

SQLite tables (builder owns exact DDL):

- `media`: id (xxh3-128 hex per the identify step, primary key), slug (unique, 11
  chars base62 derived from the hash, extended on collision), kind
  (video|image), path (absolute), source (scan|upload), folder, extension,
  size, available, title, description, public (bool), created_at,
  updated_at, recorded_at, duration_ms, width, height, codec_video,
  codec_audio, fps, bitrate, probe_json, has_poster, has_preview,
  processing_state, processing_error.
- `rendition`: media_id, height (480|720|1080), path, size, state
  (queued|running|ready|failed|cancelled), progress, error, started_at,
  finished_at.
- `tag`: id, name (unique, case-insensitive), colour. `media_tag` link.
- `view`: media_id, ip_hash, first_seen. Count is `COUNT(*)` per media.
- `upload_token`: id, name, prefix, token_hash, created_at, last_used_at,
  revoked_at.
- `session`: id, csrf, created_at, expires_at. Sessions persist across
  restarts (unlike today).
- `setting`: key, value_json. Admin-edited overrides.
- `legacy_id`: old_id, media_id, for redirecting pre-rewrite links.
- `job`: id, media_id, kind (probe|poster|preview|thumb|transcode:720),
  state, attempts, error, created_at, updated_at. The queue is in the DB so
  it survives restarts.

### Processing pipeline

1. **Discover**: at startup, every scan interval (default 5 min, 0 off), on
   the admin "Scan now" button, on filesystem events (debounced), and
   immediately after an upload. Walk configured folders recursively for the
   allowed extensions (video: mp4, m4v, mov, webm, mkv; image: png, jpg,
   jpeg, webp, gif). Skip files still growing (size unchanged for 2 s, or
   mtime older than 10 s).
2. **Identify**: xxh3-128 over the first 16 MiB, the last 16 MiB, and the
   file size (little-endian u64 appended). My call: the desktop app trims
   clips, and two exports from one recording trimmed only at the end share
   an identical head, so a head-only hash like Fireshare's would merge
   them. Byte-identical files still get the same ID. Same hash already
   known at another path: if the old path is gone, relink; else skip as a
   duplicate. Files that vanish are marked unavailable, not deleted from
   the DB, so re-appearing files keep their metadata and views.
3. **Insert** with title = file stem, public = the configured default for
   its source, recorded_at parsed from the filename (desktop app pattern
   and common OBS patterns), falling back to probe creation tags, then
   mtime.
4. **Jobs**: enqueue probe, poster, preview (videos) or thumb (images).
   Transcode enqueueing depends on the mode:
   - `off`: never automatically; admin can still queue by hand.
   - `compat` (default): only when the original is not browser-playable
     (container not mp4/webm, or video codec not h264/vp9/av1, or audio
     not aac/opus/mp3). Then a single 720p (or the highest ladder rung at
     or below the source height) is made so the clip page, the native
     player and Discord embeds work. Plain mp4 uploads from the app cost
     nothing.
   - `on`: every rung of the ladder at or below the source height, for
     every new video.
   The clip page, `og:video` and the RSS enclosure point at the best
   browser-playable file: the original when it is playable, else the
   highest ready rendition, else `og:video` is omitted and only
   `og:image` is sent. A worker pool (default 1 for
   transcodes, 2 for cheap jobs) pulls from the queue. Each ffmpeg run
   writes to a temp file then renames, is killed on timeout, and reports
   progress parsed from `-progress pipe:1`.
5. **Commands** (reference, builder may tune):
   - probe: `ffprobe -v error -print_format json -show_format -show_streams`.
   - poster: `ffmpeg -y -ss {10% of duration} -i src -frames:v 1 -vf scale=-2:720 poster.jpg`, retry at `-ss 0` on failure.
   - preview: `-ss {mid} -t 4 -vf scale=-2:360 -an -c:v libx264 -preset veryfast -crf 30 -movflags +faststart preview.mp4`. Skipped when x264 is unavailable.
   - transcode: `-vf scale=-2:{h}` with encoder-specific args, `-c:a aac -b:a 128k -movflags +faststart`, bitrate caps per height.
   - image thumb: `image` crate, 720px on the long edge, webp.
6. **Encoder detection** at startup and on demand: run `ffmpeg -encoders`,
   then a 1-second test encode per candidate (h264_nvenc, h264_vaapi,
   h264_qsv, h264_videotoolbox, libx264) and keep the first that succeeds.
   The result is shown in the admin stats row and `check` output.
7. **Webhooks** fire once per media when it first becomes public and has a
   poster. Discord gets an embed with title, link, poster. Generic gets the
   media JSON.
8. **Missing ffmpeg**: jobs stay queued, the admin stats row shows "ffmpeg
   not found", library and clip pages render with a placeholder poster and
   the native `<video>` element plays the original. The server never
   refuses to start over it.

### Serving media

- `/media/{slug}/original.{ext}`, `/media/{slug}/{480,720,1080}p.mp4`,
  `/media/{slug}/poster.jpg`, `/media/{slug}/preview.mp4`,
  `/media/{slug}/thumb.webp`. All support HTTP range requests, ETag,
  `Content-Disposition: inline` with the original filename on the original.
- Only files recorded in the DB are served, resolved from DB paths, never
  from request input. Symlinks are allowed only inside the configured
  folders (users symlink libraries), resolved paths must stay within a
  configured folder or the data dir.
- Link-only items are served to anyone holding the slug. Slugs carry ~64
  bits of entropy from the hash. That is the privacy model: same as today.
- `/files/{old_id}` redirects (301) to the new clip page via `legacy_id`.

### Auth and security

- Admin password from `ADMIN_PASSWORD` or an argon2 hash from
  `ADMIN_PASSWORD_HASH`. Login rate limited per IP (10/min) and globally.
- Session cookie `fthr_admin`, HttpOnly, SameSite=Lax (Strict breaks the
  redirect back from a shared link), Secure when the scheme or trusted
  forwarded proto is https. 8 h lifetime, sliding. CSRF token per session,
  required on every state-changing admin request as `X-CSRF-Token` or a
  form field.
- Upload rate limit per token (30/min) and body limit `MAX_UPLOAD_BYTES`
  (default 500 MiB), streamed to disk, never buffered in memory.
- Security headers as today: nosniff, `X-Frame-Options: DENY` except on
  clip pages (Discord needs none; keep DENY for admin), `Referrer-Policy`,
  CSP that allows only self (fonts are bundled, so no Google Fonts
  exception), HSTS only on https.
- Admin HTML is rendered server-side; all user strings go through
  template escaping. No `innerHTML` with user data in the JS layer.

### Frontend

- Askama templates: `base`, `library`, `clip`, `image`, `admin/*`, `login`.
- One stylesheet with the 2.0 tokens: backgrounds `#070a09` / `#0a0f0d` /
  `#101512` / `#141b17`, borders `#1b231e` / `#242c27` / `#49584f`, text
  `#f3f6f4` / `#a0aba5` / `#7e8a83`, accent `#20e3a0` with dim `#16bf87`
  and soft `#0d2b20`, error `#ff795d`, warning `#e7b56e`, delete
  `#b84f45`. Radii 4/6/8, 8px spacing grid, title strip 36px, rail 94px,
  controls 38px, page inset 24px, 120 ms hover fades, 16:9 cards 250 to 360
  px wide.
- Fonts bundled: Oswald Regular/SemiBold/Bold for display and labels, Inter
  for body (both OFL 1.1).
- Icons: the 2.0 app's SVG set from `FTHR_UI/assets/icons/` at tag
  v2.0.0-alpha (clips, settings, upload, copylink, delete, play, pause,
  chevrons, plus, refresh, close), inlined as a sprite and tinted with
  CSS. Glyphs it lacks (image, rss, tag, search, eye, check) come from
  Lucide (ISC), drawn at the same stroke weight. The FTHR logo sits in the
  title strip; the owner is an FTHR maintainer.
- JS, no framework, no build: hover preview playback, copy link, player
  quality switching (source selection by `<source>` list and
  `canPlayType`), `?t=` seek, admin bulk selection and actions via
  `fetch` against `/admin/api/*`, processing queue refresh via
  Server-Sent Events at `/admin/api/events` with a polling fallback.
- Mobile: the rail collapses to a bottom bar below 700px; grid drops to one
  column below 480px.
- Light theme: not in scope. One dark theme.

### Routes (summary)

Public: `GET /`, `GET /images`, `GET /w/{slug}`, `GET /i/{slug}`,
`GET /media/{slug}/{file}`, `GET /feed.rss`, `GET /healthz`,
`GET /files/{old_id}` (redirect), `GET /api/library?kind=&q=&tag=&sort=&page=`
(JSON, public items only).

Upload: `HEAD /upload`, `POST /upload`.

Admin pages: `GET /admin/login`, `POST /admin/login`, `POST /admin/logout`,
`GET /admin`, `GET /admin/{files|tags|library|transcoding|sharing|integrations|tokens}`.

Admin API (cookie + CSRF): `GET/PATCH /admin/api/media/{id}`,
`POST /admin/api/media/bulk` (action + ids), `DELETE /admin/api/media/{id}`,
`POST /admin/api/media/{id}/transcode`, `DELETE /admin/api/media/{id}/renditions/{h}`,
`GET/POST/PATCH/DELETE /admin/api/tags`, `GET/PUT /admin/api/settings`,
`GET/POST/DELETE /admin/api/tokens`, `POST /admin/api/scan`,
`GET /admin/api/jobs`, `POST /admin/api/jobs/{id}/{cancel|retry}`,
`GET /admin/api/events` (SSE), `GET /admin/api/stats`.

### Configuration

Precedence: admin-edited settings (DB) > env vars > `config.toml` > defaults.
Env vars: the existing seven plus `DATA_DIR`, `VIDEO_DIRS`, `IMAGE_DIRS`,
`ADMIN_PASSWORD_HASH`, `TRUSTED_PROXIES`, `SCAN_INTERVAL_MINUTES`,
`TRANSCODING` (off|compat|on), `ENCODER` (auto|x264|nvenc|vaapi|qsv|videotoolbox),
`TRANSCODE_WORKERS`, `TRANSCODE_TIMEOUT_MINUTES`, `FFMPEG_PATH`,
`FFPROBE_PATH`, `DISCORD_WEBHOOK_URL`, `WEBHOOK_URL`, `SITE_TITLE`,
`LOG_LEVEL`.

### Migration from the current service

`fthr-clips-server import-legacy` (also run automatically on first start if
`DATA_DIR` contains `<id>.json` sidecars and no `library.db`): for each
sidecar, move the raw file to `uploads/<original name>` (dedupe by suffix),
insert the media row with the sidecar's `listed` as `public`, record the old
id in `legacy_id`, and queue processing. Old `/files/{id}` links keep
working through the redirect. The old files are moved, not copied, so disk
use does not double.

### Deploy

- Docker image: multi-stage, final stage `debian:bookworm-slim` with
  `ffmpeg` from apt, non-root uid 10001, `/data` volume, `EXPOSE 8080`,
  published for `linux/amd64` and `linux/arm64`. GPU variants are out of
  scope for the image; users with NVENC run the bare binary or mount their
  own ffmpeg.
- The existing compose file works unchanged. The README will recommend
  raising `mem_limit` to 2g and adding `/videos` and `/images` mounts when
  transcoding is on. `read_only: true` still works because everything
  writable is under `/data` or `/tmp`.
- GitHub Actions: `cargo test` on Linux, macOS, Windows; release workflow
  builds the five targets and the multi-arch image on tags.
- Bare metal: one binary, `DATA_DIR=./data ADMIN_PASSWORD=... UPLOAD_TOKEN=... ./fthr-clips-server`.

### Tests

- Integration tests through axum's test harness covering every behaviour
  the current 12 pytest tests cover (bearer auth on POST and HEAD, upload
  round trip, 413 with partial cleanup, 429, listing, admin login, list and
  unlist, delete then 404, filename escaping, logout invalidation,
  dashboard gating), plus: hash dedupe, unavailable-then-relink, slug
  collision, legacy import and redirect, OpenGraph tags on the clip page,
  RSS validity, view dedupe by IP, CSRF rejection, range requests, link-only
  items absent from library, search and feed.
- Pipeline tests run against a tiny generated video fixture and are skipped
  with a clear message when ffmpeg is absent, so CI without ffmpeg still
  passes the rest.

## Repository plan

- New branch `rust-rewrite`. First commit deletes `fthr_server.py`,
  `tests/`, `pyproject.toml`, `requirements*.txt`, the Python `Dockerfile`,
  and adds the Cargo workspace skeleton. Nothing from the Python code is
  ported; the pytest cases are re-read as a spec only.
- Layout: `Cargo.toml`, `src/{main.rs,config.rs,db/,web/,media/,jobs/,
  ffmpeg/,templates via askama}`, `migrations/`, `static/`, `templates/`,
  `tests/`, `Dockerfile`, `docker-compose.yml`, `.github/workflows/`,
  `README.md`, `LICENSE`, `docs/`.
- Add `.superpowers/` to `.gitignore`.

## Decisions log

Owner's decisions:

- Rust, Fireshare-like features, 2.0 look, portability as a goal.
- Transcoding, images and RSS are in. Multi-user, MFA, games, profiles,
  editing are out.
- License is GPL-3.0-only.
- The three builder questions below: "go with what Fireshare does or what
  you recommend".

My calls, so the owner can check them:

- Transcoding default is `compat` (Fireshare defaults to off; compat is
  the recommendation because it makes mkv and HEVC share links work on a
  fresh install without burning CPU on ordinary mp4s).
- "Library requires sign-in" hides listings only; share links keep
  working.
- Delete removes source files only under `uploads/`; scanned-folder files
  need an explicit tick. Fireshare deletes the file unconditionally; the
  conservative version is the recommendation because those folders are the
  user's own recordings.
- Content ID hashes head, tail and size instead of Fireshare's head only.
- Duplicate uploads return the existing item.
- 415 exists for types the desktop app never sends.
- Icons come from the 2.0 app, gaps filled from Lucide.
- README keeps a one-line note: built by an FTHR maintainer, not an
  official FTHR Community release. Drop it if that is wrong.
- Session cookie is SameSite=Lax rather than Strict.
- Fonts are bundled, no Google Fonts.
- Sessions, jobs and settings live in SQLite so restarts lose nothing.

## Left to the builder

- Exact DDL, query layer choice between sqlx and rusqlite, migration tool.
- Precise ffmpeg argument tuning, bitrate caps, and the encoder test clip.
- Slug derivation details beyond "base62 from the hash, 11 chars, extend on
  collision".
- Template structure, CSS organisation, SVG sprite build.
- SSE vs polling implementation details.
- CI matrix specifics, cross-compilation tooling (cross, cargo-zigbuild).
- Filename date parsing patterns.
- How the mobile bottom bar looks.

## Recorded deviations

Differences between this spec and the implementation, accepted during final review:

- Image thumbnails are JPEG (`thumb.jpg`), not WebP (`thumb.webp`).
- Scanned files default to link-only (not listed publicly) until published in the admin area.
- Scan folder paths are configured only through environment variables or `config.toml`; the Library admin tab shows them read-only.
