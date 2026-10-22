# FTHR Clips Server

A self-hosted clip sharing server for the FTHR desktop app: upload videos and images, get a link, browse a library, and optionally transcode for playback everywhere. A single Rust binary with an embedded SQLite database.

> Built by an FTHR maintainer; not an official FTHR Community release.

## Features

- Upload endpoint for the FTHR desktop app (`POST /upload`, bearer token auth).
- Public library of videos and images with tags, search, sorting, thumbnails and an RSS feed (`/feed.rss`).
- Clip and image pages (`/w/<slug>`, `/i/<slug>`) with link previews.
- Admin area (`/admin`) for files, tags, library, transcoding, sharing, integrations and upload tokens.
- Optional transcoding to a web-compatible 720p rendition, with automatic encoder detection.
- Scan folders: index existing video and image folders without copying them.
- Discord and generic webhooks when a clip is first made public.
- One-command import from the old Python version; old `/files/` links keep working.
- Small static binary builds for Linux (musl), macOS and Windows, plus a multi-arch Docker image.

## Quick start

Both `UPLOAD_TOKEN` and an admin password are needed. Generate secrets with `openssl rand -base64 32`.

### Binary

Download the archive for your platform from the GitHub releases page (it contains the binary, `README.md`, `LICENSE` and `config.example.toml`), then:

```sh
UPLOAD_TOKEN=change-me ADMIN_PASSWORD=change-me-too ./fthr-clips-server serve
```

It listens on `127.0.0.1:8080` and stores data in `./data`. Install `ffmpeg` and `ffprobe` for thumbnails and transcoding (without them, uploads still work as link-only files).

### Docker

```sh
docker run -d --name fthr-clips \
  -e UPLOAD_TOKEN=change-me -e ADMIN_PASSWORD=change-me-too \
  -e PUBLIC_BASE_URL=https://clips.example.com \
  -p 127.0.0.1:18080:8080 -v clips-data:/data \
  ghcr.io/<owner>/fthr-clips-server:latest
```

The image runs as uid 10001, bundles `ffmpeg`, stores everything in `/data` and has a health check on `/healthz`. To build it yourself: `docker build -t fthr-clips-server .`

### Docker Compose

```sh
cp .env.example .env     # then set UPLOAD_TOKEN, ADMIN_PASSWORD and PUBLIC_BASE_URL
docker compose up -d --build
```

The service is published on `127.0.0.1:18080`; put a reverse proxy in front (see below). Uncomment the `/videos` and `/images` mounts in `docker-compose.yml` and set `VIDEO_DIRS=/videos` / `IMAGE_DIRS=/images` in `.env` to use scan folders. `mem_limit` is 2g; 1g is enough with `TRANSCODING=off`.

## Desktop app setup

In the FTHR desktop app choose the "Your server" provider and enter:

- URL: `https://clips.example.com/upload`
- Authorization: `Bearer <your UPLOAD_TOKEN>`

Additional upload tokens can be created in `/admin/tokens`.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `UPLOAD_TOKEN` | none (required unless `config.toml` exists) | Bearer token the desktop app uses to upload. |
| `ADMIN_PASSWORD` | none | Plain admin password. Prefer the hash below. |
| `ADMIN_PASSWORD_HASH` | none | Argon2id hash of the admin password (`fthr-clips-server hash-password`). |
| `DATA_DIR` (alias `STORAGE_DIR`) | `data` (`/data` in Docker) | Database, uploads, thumbnails, renditions and `config.toml`. |
| `VIDEO_DIRS` | none | Comma-separated folders to scan for videos. |
| `IMAGE_DIRS` | none | Comma-separated folders to scan for images. |
| `PUBLIC_BASE_URL` | none | Public URL used in links, feed and webhooks, for example `https://clips.example.com`. |
| `MAX_UPLOAD_BYTES` | `524288000` | Maximum upload size in bytes. |
| `BIND_HOST` | `127.0.0.1` (`0.0.0.0` in Docker) | Address to listen on. |
| `PORT` | `8080` | Port to listen on. |
| `TRUSTED_PROXIES` | none | Comma-separated IPs or CIDRs of reverse proxies whose `X-Forwarded-For` is trusted. |
| `SCAN_INTERVAL_MINUTES` | `15` | How often scan folders are rescanned. |
| `TRANSCODING` | `compat` | `off`, `compat` or `on`. See Transcoding. |
| `ENCODER` | `auto` | `auto`, `x264`, `nvenc`, `vaapi`, `qsv` or `videotoolbox`. |
| `TRANSCODE_WORKERS` | `1` | Number of concurrent transcodes. |
| `TRANSCODE_TIMEOUT_MINUTES` | `60` | Per-job transcode timeout. |
| `FFMPEG_PATH` | found on `PATH` | Path to the `ffmpeg` binary. |
| `FFPROBE_PATH` | found on `PATH` | Path to the `ffprobe` binary. |
| `DISCORD_WEBHOOK_URL` | none | Discord webhook notified when a clip is first made public. |
| `WEBHOOK_URL` | none | Generic JSON webhook notified at the same moment. |
| `SITE_TITLE` | `FTHR Clips` | Site name shown in the UI, feed and previews. |
| `LOG_LEVEL` | `info` | Log filter (`error`, `warn`, `info`, `debug`, or an `EnvFilter` string). |

Precedence: settings edited in the admin pages (stored in the database) override environment variables, which override `{DATA_DIR}/config.toml`, which overrides the built-in defaults. `config.toml` uses the lowercase variable names; see `config.example.toml`. The admin pages `/admin/library`, `/admin/transcoding`, `/admin/sharing` and `/admin/integrations` edit the settings that can be overridden (site title, public URL, scan interval, transcoding, encoder, workers, timeout, webhooks and more). Startup settings (`DATA_DIR`, `BIND_HOST`, `PORT`, scan folders, credentials, `TRUSTED_PROXIES`, `MAX_UPLOAD_BYTES`) come only from the environment or `config.toml`.

## Transcoding

- `off`: files are served as uploaded.
- `compat`: only files browsers may not play (for example HEVC or unusual containers) get a compatible H.264 rendition.
- `on`: every video gets a rendition.

With `ENCODER=auto` the server picks the best working encoder found in `ffmpeg` (hardware encoders first, `x264` as the fallback). Run `fthr-clips-server check` to see what was detected. The Docker image has software `x264` only; for NVENC, VAAPI or QSV run the binary directly (or mount your own `ffmpeg` and set `FFMPEG_PATH`). On ARM (Raspberry Pi and similar) software encoding is slow: keep `TRANSCODING=off` or `compat` and `TRANSCODE_WORKERS=1`.

## Scan folders

Set `VIDEO_DIRS` and/or `IMAGE_DIRS` to index existing folders in place. Files are never copied or modified; scanned items default to link-only (not listed publicly) until you publish them in the admin area. Folders are watched for changes and rescanned every `SCAN_INTERVAL_MINUTES`. Run a one-off scan with `fthr-clips-server scan`. In Docker, mount the folders read-only (for example `/videos:ro`).

## Reverse proxy

Run the server behind a TLS-terminating proxy; the server itself speaks plain HTTP. Set `PUBLIC_BASE_URL` to the public HTTPS address so generated links and the feed are correct, and set `TRUSTED_PROXIES` to your proxy's address so the client IP (admin login throttling, view counting) comes from `X-Forwarded-For` and the scheme from `X-Forwarded-Proto`, rather than being the proxy's. Do not list addresses that are not your proxy.

Caddy example:

```
clips.example.com {
    request_body {
        max_size 500MB
    }
    reverse_proxy 127.0.0.1:18080
}
```

With the compose file above, the proxy runs on the host and `TRUSTED_PROXIES=172.16.0.0/12,127.0.0.1` covers the Docker bridge gateway.

## Migrating from the Python version

Point the new server at the old data directory (for example the same Docker volume) and run:

```sh
fthr-clips-server import-legacy
```

For each old `<id>.json` sidecar the file is moved into `uploads/` under its original name, added to the library (the old "listed" flag becomes public), and queued for thumbnailing and transcoding. Old `/files/<id>` links redirect to the new pages. Files are moved, not copied, so disk use does not double. The import also runs automatically on first start when the data directory contains legacy sidecars and no `library.db`.

Notes: musl release builds work without system libraries because SQLite is bundled and TLS uses rustls (no OpenSSL).

## CLI subcommands

| Command | What it does |
| --- | --- |
| `fthr-clips-server serve` | Run the web server (default when no subcommand is given). |
| `fthr-clips-server scan` | Scan the configured folders once, print the report and exit. |
| `fthr-clips-server import-legacy` | Import the old Python service's files from the data dir and exit. |
| `fthr-clips-server hash-password` | Read a password from stdin and print its Argon2id hash for `ADMIN_PASSWORD_HASH`. |
| `fthr-clips-server check` | Print diagnostics: data dir, database, ffmpeg, encoder, scan folders, bind address. |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo run -- check
```

Tests in `tests/ffmpeg.rs` and `tests/transcode.rs`, and the media parts of `tests/jobs.rs`, need `ffmpeg` and `ffprobe`; without them they skip. CI runs Linux with ffmpeg, and macOS and Windows without it to exercise the skip paths. CI also builds the Docker image; pushing a `v*` tag builds release archives for five targets and publishes the multi-arch image to GHCR.

## License

GPL-3.0-only. See `LICENSE`.
