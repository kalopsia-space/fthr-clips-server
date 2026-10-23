# Development

## Building

A stable Rust toolchain is all you need. SQLite is bundled and TLS uses rustls, so there are no system libraries to install.

```sh
cargo build --release
./target/release/fthr-clips-server check
```

## Tests

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

The suite spins up real servers on random ports and drives them over HTTP, including uploads, admin sessions, scanning and the job queue. Tests that need `ffmpeg` and `ffprobe` (`tests/ffmpeg.rs`, `tests/transcode.rs` and the media parts of `tests/jobs.rs`) skip with a message when they are not installed. The transcode tests encode real video and take a minute or two.

## Command line

| Command | What it does |
|---|---|
| `fthr-clips-server serve` | Run the server. This is the default when no subcommand is given. |
| `fthr-clips-server check` | Print what the server sees: data directory, database, `ffmpeg`, detected encoder, scan folders, bind address. |
| `fthr-clips-server scan` | Walk the scan folders once, print a report and exit. |
| `fthr-clips-server import-legacy` | Import a Python-era data directory and exit. |
| `fthr-clips-server hash-password` | Read a password on stdin and print its Argon2id hash for `ADMIN_PASSWORD_HASH`. |

## Layout

```
src/
  app.rs          router, state, startup
  config.rs       environment and config.toml
  settings.rs     runtime settings with admin overrides
  db/             SQLite queries, one module per table family
  media/          content ids, type sniffing, scanning, derived paths
  ffmpeg/         locating ffmpeg, probing, encoder detection
  jobs/           queue, workers, poster/preview/thumb/transcode jobs
  web/            handlers: upload, media files, library, clip pages, feed, admin/
  webhooks.rs     Discord and generic notifications
  legacy.rs       import of the Python layout
migrations/       schema, applied automatically
templates/        Askama templates
static/           CSS, JS, fonts, icons (embedded in the binary)
tests/            integration tests, one file per area
```

The frontend is server-rendered HTML with a small amount of plain JavaScript and no build step. The content security policy forbids inline scripts and styles, so new UI goes in `static/app.js` and `static/app.css`.

## Releases

Pushing a tag like `v2.1.0` runs the release workflow: it cross-compiles static binaries for `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, attaches them to a GitHub release, and publishes a `linux/amd64` + `linux/arm64` image to `ghcr.io/kalopsia-space/fthr-clips-server`. Every release moves the `edge` image tag; tags without a pre-release suffix also move `latest` and the `X.Y` tag.

Linux and Docker are the supported platforms. The code keeps its platform gates and may well build elsewhere, but nothing else is tested.

## Design history

The rewrite's design document, implementation plan and a list of known small follow-ups live in [`superpowers/`](superpowers/).
