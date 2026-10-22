<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/logo-dark.png">
  <img src="docs/images/logo-light.png" alt="FTHR Clips" width="96">
</picture>

# FTHR Clips Server

**Your own clip site. One binary, one folder, your rules.**

Upload from the FTHR Clips desktop app, get a link that unfurls in Discord, and browse everything you've ever captured in a library that looks like the app you recorded it with.

[![CI](https://github.com/kalopsia-space/fthr-clips-server/actions/workflows/ci.yml/badge.svg)](https://github.com/kalopsia-space/fthr-clips-server/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/kalopsia-space/fthr-clips-server?include_prereleases&label=release)](https://github.com/kalopsia-space/fthr-clips-server/releases)
[![Docker](https://img.shields.io/badge/ghcr.io-fthr--clips--server-2496ED?logo=docker&logoColor=white)](https://github.com/kalopsia-space/fthr-clips-server/pkgs/container/fthr-clips-server)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-20e3a0)](LICENSE)

[Quick start](#quick-start) · [How it works](#how-it-works) · [Docs](docs/README.md) · [Desktop app setup](docs/desktop-app.md)

</div>

> Built by FTHR's Linux branch maintainer as a side project. Not an official FTHR Community release.

---

## Why this exists

Clip hosts come and go, links rot, and the free tiers compress your 1440p ace into soup. FTHR Clips Server is the "Your server" option in the desktop app's upload menu, grown into a real home for your clips:

- **It's yours.** Runs on a Raspberry Pi, an old laptop, a free Oracle ARM box or any VPS. Your files stay on your disk.
- **It just works with the app.** Point FTHR Clips at it once; every upload comes back with a share link.
- **Links look good.** Paste one in Discord and the clip plays inline, with a poster and title.
- **It stays out of the way.** No database server, no Node, no build step. Install `ffmpeg` if you want thumbnails and transcoding; skip it and everything still serves.

## What you get

| | |
|---|---|
| **A library you'll actually browse** | Clips grouped by day, posters that play on hover, tags, search and sort, a separate screenshots view. Styled after the FTHR Clips 2.0 desktop UI: near-black green surfaces, mint accent, Oswald type. |
| **Share pages that embed** | Every clip and image has a short link (`/w/abc123`, `/i/abc123`) with OpenGraph tags, so Discord, Slack and friends show the video inline. |
| **Public or link-only** | Each item is either listed in the library or reachable only by its link. Scanned folders start link-only, so nothing leaks by accident. |
| **Transcoding that knows when to bother** | The default `compat` mode leaves normal MP4s alone and only re-encodes files browsers can't play (MKV, HEVC). Hardware encoders are detected automatically. |
| **Bring your existing folders** | Point it at the folders where the app already saves recordings. Files are indexed in place, never copied or modified, and new captures show up on their own. |
| **An admin area, not a config file** | Files with bulk actions, tags, transcoding, sharing, webhooks and upload tokens, all behind a single password with sessions and CSRF protection. |
| **The small stuff** | RSS feed of public clips, Discord and generic webhooks when something goes public, view counts, a one-command import from the old Python server. |

## Quick start

You need two secrets: an **upload token** for the desktop app and an **admin password** for you. Make them with `openssl rand -base64 32`.

### Docker (recommended)

```sh
docker run -d --name fthr-clips \
  -e UPLOAD_TOKEN=paste-a-long-random-token \
  -e ADMIN_PASSWORD=paste-another-one \
  -e PUBLIC_BASE_URL=https://clips.example.com \
  -p 127.0.0.1:18080:8080 \
  -v clips-data:/data \
  ghcr.io/kalopsia-space/fthr-clips-server:latest
```

The image bundles `ffmpeg`, runs as a non-root user, keeps everything under `/data` and has a health check. Put a reverse proxy with TLS in front of port 18080 and you're done. There's a [`docker-compose.yml`](docker-compose.yml) with sensible hardening if you prefer compose.

### Single binary

Grab the Linux archive for your CPU (x86_64 or aarch64) from the [releases page](https://github.com/kalopsia-space/fthr-clips-server/releases), then:

```sh
UPLOAD_TOKEN=paste-a-long-random-token \
ADMIN_PASSWORD=paste-another-one \
./fthr-clips-server serve
```

It listens on `127.0.0.1:8080` and stores data in `./data`. Install `ffmpeg` for posters and transcoding.

### Connect the desktop app

In FTHR Clips, open the upload settings, choose **Your server**, and enter:

| Field | Value |
|---|---|
| URL | `https://clips.example.com/upload` |
| Authorization | `Bearer paste-a-long-random-token` |

Hit **Test connection**, record something, hit share. The link in your clipboard is your new server. More detail in [Desktop app setup](docs/desktop-app.md).

## How it works

```mermaid
flowchart LR
    App[FTHR Clips app] -- "POST /upload" --> S[(fthr-clips-server)]
    Folders[Your recording folders] -- "watched & scanned" --> S
    S --> Lib[Library & share pages]
    S --> Jobs[ffmpeg jobs: posters, previews, transcodes]
    Lib -- "og:video" --> Discord[Discord embeds]
    S -- "webhook" --> Discord
```

Everything lives in one data directory: a SQLite database, the uploaded files, and a `derived/` folder of posters, previews and renditions. Files are identified by content hash, so re-uploading the same clip gives you the same link, and a file you move on disk keeps its page, tags and view count. Processing runs in a small job queue that survives restarts and simply waits when `ffmpeg` isn't installed.

## Documentation

| Guide | What's in it |
|---|---|
| [Deployment](docs/deployment.md) | Docker, Compose, reverse proxies, `PUBLIC_BASE_URL` and `TRUSTED_PROXIES`, running on ARM. |
| [Configuration](docs/configuration.md) | Every setting, its default, and which ones the admin pages can change at runtime. |
| [Library, scanning and transcoding](docs/media.md) | Scan folders, visibility rules, the three transcoding modes, encoder detection. |
| [Desktop app setup](docs/desktop-app.md) | Connecting FTHR Clips, extra upload tokens, what the app expects from the server. |
| [Migrating from the Python server](docs/migration.md) | The one-command import and how old links keep working. |
| [Development](docs/development.md) | Building, testing, the CLI, and how releases are made. |

## Status

This is an alpha. The server has a thorough test suite and has been reviewed end to end, but it is young: expect rough edges in the admin UI and please [open an issue](https://github.com/kalopsia-space/fthr-clips-server/issues) when you find one. Linux (x86_64 and aarch64) and Docker are the supported platforms.

## License

[GPL-3.0-only](LICENSE), the same license as FTHR Clips. Fonts are Oswald and Inter under the SIL Open Font License.
