# Migrating from the Python server

The previous version of this project was a small Python service that stored each upload as `{id}` plus an `{id}.json` sidecar in its data directory and served them at `/files/{id}`. The Rust server imports that layout and keeps the old links alive.

## What happens

Point the new server at the old data directory. The same Docker volume, the same `STORAGE_DIR` (still accepted as an alias of `DATA_DIR`), whatever you had. On start, and again on every start while anything remains, the server:

1. Finds each `{id}` / `{id}.json` pair.
2. Moves the file into `uploads/` under its original name. Moved, not copied, so disk use does not double.
3. Adds it to the library. The old `listed` flag becomes public or link-only.
4. Records the old id, so `/files/{id}` redirects to the clip's new page.
5. Queues the poster, probe and (if needed) transcode jobs.
6. Deletes the sidecar.

Files the server cannot recognise as a video or image are left in place, logged once, and not retried.

## Running it by hand

```sh
fthr-clips-server import-legacy
```

does the same thing and prints a count. It is safe to run repeatedly.

## Settings that carried over

`UPLOAD_TOKEN`, `ADMIN_PASSWORD`, `STORAGE_DIR`, `PUBLIC_BASE_URL`, `MAX_UPLOAD_BYTES`, `BIND_HOST` and `PORT` mean the same thing they did before. The old `docker-compose.yml` starts the new image without edits, though you will want the new file for its scan-folder mounts and the larger memory limit.

## What changed

- The admin session cookie is now stored in the database, so logging in survives restarts.
- Share links are `/w/{slug}` and `/i/{slug}` instead of `/files/{id}`. Old links redirect permanently.
- Clips can be public or link-only per item, instead of one global list.
