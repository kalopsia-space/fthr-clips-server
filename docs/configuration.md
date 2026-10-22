# Configuration

Settings come from four places. Later ones win:

1. Built-in defaults.
2. `config.toml` in the data directory (keys are the lowercase variable names; see [`config.example.toml`](../config.example.toml)).
3. Environment variables.
4. The admin pages, for the settings that can change at runtime. These are stored in the database and survive restarts.

Secrets belong in the environment or a file with tight permissions, not in the admin UI.

## Startup settings

These are read once when the server starts.

| Variable | Default | What it does |
|---|---|---|
| `UPLOAD_TOKEN` | none | Bearer token the desktop app sends. Required unless a `config.toml` exists. More tokens can be created in **Admin → Tokens**. |
| `ADMIN_PASSWORD` | none | Admin password in plain text. Fine for a home server; prefer the hash for anything shared. |
| `ADMIN_PASSWORD_HASH` | none | Argon2id hash of the admin password. Make one with `fthr-clips-server hash-password`. Takes precedence over `ADMIN_PASSWORD`. |
| `DATA_DIR` | `data` (`/data` in Docker) | Where the database, uploads, derived files and `config.toml` live. `STORAGE_DIR` is accepted as an alias for compatibility with the Python version. |
| `VIDEO_DIRS` | none | Comma-separated folders to index for videos. See [scanning](media.md#scan-folders). |
| `IMAGE_DIRS` | none | Comma-separated folders to index for images. |
| `BIND_HOST` | `127.0.0.1` (`0.0.0.0` in Docker) | Address to listen on. |
| `PORT` | `8080` | Port to listen on. |
| `TRUSTED_PROXIES` | none | IPs or CIDRs of reverse proxies whose forwarded headers are trusted. See [deployment](deployment.md#reverse-proxy-and-public-url). |
| `MAX_UPLOAD_BYTES` | `524288000` (500 MiB) | Largest accepted upload. Larger requests get a 413 and nothing is written. |
| `FFMPEG_PATH` | found on `PATH` | Explicit path to `ffmpeg`. |
| `FFPROBE_PATH` | found on `PATH` | Explicit path to `ffprobe`. |
| `LOG_LEVEL` | `info` | `error`, `warn`, `info`, `debug`, or a full `tracing` filter string such as `info,fthr_clips_server::jobs=debug`. |

## Runtime settings

These have a default from the environment and can be changed in the admin area without a restart.

| Variable | Default | Admin page | What it does |
|---|---|---|---|
| `SITE_TITLE` | `FTHR Clips` | Sharing | Name shown in the UI, page titles, feed and link previews. |
| `PUBLIC_BASE_URL` | none | Sharing | Public address used in every generated link. |
| `SCAN_INTERVAL_MINUTES` | `5` | Library | How often scan folders are re-walked. `0` disables the timer; the filesystem watcher still runs. |
| `TRANSCODING` | `compat` | Transcoding | `off`, `compat` or `on`. See [transcoding](media.md#transcoding). |
| `ENCODER` | `auto` | Transcoding | `auto`, `x264`, `nvenc`, `vaapi`, `qsv` or `videotoolbox`. |
| `TRANSCODE_WORKERS` | `1` | Transcoding | Concurrent transcodes. Keep at 1 unless you have cores to spare. |
| `TRANSCODE_TIMEOUT_MINUTES` | `60` | Transcoding | A transcode running longer than this is killed and marked failed. |
| `DISCORD_WEBHOOK_URL` | none | Integrations | Discord webhook that receives an embed when a clip first becomes public. |
| `WEBHOOK_URL` | none | Integrations | Generic endpoint that receives the clip's JSON at the same moment. |

The admin pages also hold a few settings with no environment variable: the library's default sort, whether uploads from the app start public or link-only, the upload folder name, the file extensions to scan for, and whether the library requires a login to browse (share links keep working either way).

## Example `config.toml`

```toml
public_base_url = "https://clips.example.com"
trusted_proxies = ["127.0.0.1"]
video_dirs = ["/srv/recordings"]
transcoding = "compat"
site_title = "Kalopsia's clips"
```

Place it at `{DATA_DIR}/config.toml`. The full annotated example is [`config.example.toml`](../config.example.toml).

## Checking what the server sees

```sh
fthr-clips-server check
```

prints the resolved data directory, whether the database opens, where `ffmpeg` was found and which encoder detection picked, each scan folder and whether it exists, and the bind address. Run it after any configuration change.
