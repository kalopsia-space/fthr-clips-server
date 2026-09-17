# FTHR Clips Server

A small, self-hosted upload server for the **Your server** provider in [FTHR Clips](https://github.com/FTHR-Community/FTHR-Clips).

It accepts authenticated `multipart/form-data` uploads in the `clip` field, returns a shareable URL, and provides a polished responsive gallery at `/` for browsing and sharing stored clips. It is intentionally simple, private-by-default, and suitable for running behind an HTTPS reverse proxy.

## Quick start with Docker

```bash
cp .env.example .env
# Set UPLOAD_TOKEN to a long random value in .env
docker compose up -d --build
curl -I http://127.0.0.1:8080/upload
curl -H "Authorization: Bearer $UPLOAD_TOKEN" \
  -F clip=@example.mp4 http://127.0.0.1:8080/upload
```

The server stores files in `./data` and serves them at `/files/<id>`. Do not expose port 8080 directly to the internet; put Caddy, Nginx, or another HTTPS reverse proxy in front of it.

## Configure FTHR Clips

In FTHR Clips' uploader settings:

- Provider: `Your server`
- Server URL: `https://clips.example.com/upload`
- Authorization header: `Bearer YOUR_UPLOAD_TOKEN`

The token is entered on the desktop client and must not be committed to this repository.

## Configuration

- `UPLOAD_TOKEN` — required bearer token/API key for FTHR Clips uploads.
- `ADMIN_PASSWORD` — required password for the protected `/admin` dashboard.
- `STORAGE_DIR` — directory for uploaded files; defaults to `/data` in the container.
- `PUBLIC_BASE_URL` — optional public URL used in returned links. If unset, the request's `Host` and scheme are used.
- `MAX_UPLOAD_BYTES` — maximum upload size; defaults to 524288000 (500 MiB).
- `BIND_HOST` / `PORT` — listener settings for non-container use.

## API

- `HEAD /upload` — connection test.
- `POST /upload` — authenticated multipart upload; field name must be `clip`.
- `GET /files/<id>` — download an uploaded file.
- `GET /` — responsive clip gallery with previews and copy-link controls.
- `GET /api/clips` — gallery metadata endpoint; only listed clips appear.
- `GET /admin` — password-protected admin login.
- `GET /admin/dashboard` — authenticated dashboard for listing, unlisting, and deleting clips.
- `GET /healthz` — basic health check.

Successful upload response:

```json
{"url":"https://clips.example.com/files/<id>","id":"<id>"}
```

## Development

```bash
uv venv
uv pip install --python .venv/bin/python -r requirements-dev.txt
.venv/bin/pytest -q
.venv/bin/python -m fthr_server --storage-dir ./data --token dev-token --port 8080
```

## Security notes

Use HTTPS at the reverse proxy, a long random token, a private storage directory, and a firewall rule that exposes only the proxy. Uploaded content is untrusted; the server does not execute or transcode it. Back up `data/` if uploads matter.

## License

MIT. This is an independent companion service and is not an official FTHR Community project.
