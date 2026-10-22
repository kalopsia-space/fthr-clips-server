# Deployment

The server is a single process that speaks plain HTTP on one port and keeps all of its state in one directory. Put TLS in front of it, give it a disk, and it is happy on anything from a Raspberry Pi to a VPS.

## Choosing how to run it

| | Docker image | Compose | Binary |
|---|---|---|---|
| ffmpeg included | yes | yes | install it yourself |
| Hardware encoders (NVENC, VAAPI, QSV) | no, software x264 only | no | yes, if your ffmpeg has them |
| Runs as non-root | yes (uid 10001) | yes | whatever user you start it as |
| Best for | most people | people who like one file to manage | GPU hosts, bare metal, unusual setups |

## Docker

```sh
docker run -d --name fthr-clips \
  -e UPLOAD_TOKEN=paste-a-long-random-token \
  -e ADMIN_PASSWORD=paste-another-one \
  -e PUBLIC_BASE_URL=https://clips.example.com \
  -p 127.0.0.1:18080:8080 \
  -v clips-data:/data \
  ghcr.io/kalopsia-space/fthr-clips-server:latest
```

What the image does for you:

- Bundles `ffmpeg` and `ffprobe` (software encoders).
- Runs as uid 10001 under `tini`, with a health check on `/healthz`.
- Sets `DATA_DIR=/data` and `BIND_HOST=0.0.0.0`; everything writable lives in `/data`, so `--read-only` with a `tmpfs` on `/tmp` works.
- Ships for `linux/amd64` and `linux/arm64`.

To index folders you already have, mount them read-only and name them:

```sh
  -v /srv/recordings:/videos:ro -e VIDEO_DIRS=/videos \
  -v /srv/screenshots:/images:ro -e IMAGE_DIRS=/images \
```

Build it yourself with `docker build -t fthr-clips-server .`.

## Docker Compose

```sh
cp .env.example .env    # set UPLOAD_TOKEN, ADMIN_PASSWORD, PUBLIC_BASE_URL
docker compose up -d --build
```

The included [`docker-compose.yml`](../docker-compose.yml) publishes the server on `127.0.0.1:18080`, mounts a named volume on `/data`, and applies a few hardening options: read-only root filesystem, `no-new-privileges`, a memory limit of 2 GB and a process limit of 256. If you turn transcoding off you can drop the memory limit to 1 GB. Uncomment the `/videos` and `/images` mounts to use scan folders.

## Single binary

Download the archive for your architecture from the [releases page](https://github.com/kalopsia-space/fthr-clips-server/releases). It contains the binary, this documentation, the license and an annotated `config.example.toml`.

```sh
UPLOAD_TOKEN=paste-a-long-random-token \
ADMIN_PASSWORD=paste-another-one \
./fthr-clips-server serve
```

Defaults: listens on `127.0.0.1:8080`, data in `./data`. The binaries are statically linked (musl) and need no system libraries; `ffmpeg` and `ffprobe` are found on `PATH` or through `FFMPEG_PATH` and `FFPROBE_PATH`.

A minimal systemd unit:

```ini
[Unit]
Description=FTHR Clips Server
After=network-online.target

[Service]
User=fthr
WorkingDirectory=/srv/fthr-clips
EnvironmentFile=/srv/fthr-clips/.env
ExecStart=/srv/fthr-clips/fthr-clips-server serve
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

## Reverse proxy and public URL

The server never terminates TLS. Run it behind Caddy, nginx, Traefik or similar, and tell it two things:

- **`PUBLIC_BASE_URL`**: the address people will use, for example `https://clips.example.com`. It goes into share links, OpenGraph tags, the RSS feed and webhooks. Without it the server falls back to the `Host` header, which is fine for testing and wrong behind most proxies.
- **`TRUSTED_PROXIES`**: the IP or CIDR of your proxy. Only then are `X-Forwarded-For` and `X-Forwarded-Proto` believed, which is what makes view counting, login throttling and the `Secure` cookie flag work correctly. Never list addresses that are not your proxy; anyone who can reach the port directly could otherwise spoof their IP.

Caddy, the shortest possible config:

```
clips.example.com {
    request_body {
        max_size 500MB
    }
    reverse_proxy 127.0.0.1:18080
}
```

Uploads are streamed straight to disk, but the proxy must allow bodies at least as large as `MAX_UPLOAD_BYTES` (500 MB by default).

With the Compose setup and a proxy on the host, the container sees the proxy as the Docker bridge gateway, so `TRUSTED_PROXIES=172.16.0.0/12,127.0.0.1` is the usual value.

## Running on ARM

The aarch64 binary and the arm64 image run on the Oracle Cloud free tier, Raspberry Pi 4 and 5, and similar boards. Posters and thumbnails are cheap. Software transcoding is not: keep `TRANSCODING=compat` (the default) so only files browsers cannot play get re-encoded, and leave `TRANSCODE_WORKERS=1`.

## Upgrading

The database migrates itself on start. Back up the data directory first; it is the only state there is.
