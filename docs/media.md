# Library, scanning and transcoding

## How files get in

There are two doors.

**Uploads** arrive from the desktop app (or anything that speaks the same [upload contract](desktop-app.md#the-upload-contract)) and land in the `uploads/` folder inside the data directory. The server owns these files: deleting a clip in the admin area deletes the file.

**Scan folders** are directories you already have, named in `VIDEO_DIRS` and `IMAGE_DIRS`. The server walks them on start, every `SCAN_INTERVAL_MINUTES`, and whenever the filesystem watcher sees a change. Files are indexed where they are; nothing is copied or rewritten. Deleting a scanned clip in the admin area removes it from the library and leaves the file alone unless you tick "also delete the file from disk".

Either way, every file is identified by a hash of its content. Upload the same clip twice and you get the same link. Move a scanned file to another folder and it keeps its page, tags and views. A file that disappears is marked unavailable, not forgotten, so it comes back intact if the drive does.

A file still being written (a recording in progress) is skipped and picked up on the next pass.

## Who can see what

Each clip or image is either **public** or **link-only**.

- Public items appear in the library, in search, in the RSS feed and in the public API.
- Link-only items are reachable by anyone who has the link, and nowhere else. The link is unguessable (64 bits of hash), which is the same privacy model the old Python server had.

Uploads from the app start public by default (change this in **Admin → Library**). Scanned files always start link-only, so pointing the server at your whole recordings folder never publishes anything until you say so. Flip visibility per item on its page or in bulk from **Admin → Files**.

The **Sharing** page has one more switch: "library requires sign-in". With it on, only you can browse or search, but every share link you have ever handed out keeps working, as do Discord embeds.

## Posters, previews and thumbnails

When `ffmpeg` is available, every new video gets probed for duration, resolution and codecs, a poster frame is grabbed, and a short silent preview clip is made for the hover effect in the library. Images get a thumbnail through pure Rust, no `ffmpeg` needed.

Without `ffmpeg`, the server starts and serves normally; videos show a placeholder poster and those jobs wait in the queue. Install `ffmpeg`, restart, and they run.

## Transcoding

Browsers play H.264 in MP4 or WebM. They do not play MKV containers or HEVC video, and neither does Discord's inline player. Transcoding exists to fix that, and only that.

| Mode | What happens |
|---|---|
| `off` | Files are served as they are. Share pages for unplayable files show a thumbnail and a download link instead of an inline player. |
| `compat` (default) | Nothing happens to a normal MP4. A file browsers cannot play gets one H.264 rendition at the largest rung of the ladder that fits its height (720p by default). |
| `on` | Every video gets every rung of the ladder up to its own height. Useful if you want a quality picker on every clip, expensive otherwise. |

The ladder is any subset of 480p, 720p and 1080p, set on the **Transcoding** page. Nothing is ever upscaled.

Renditions live next to the original in `derived/`, show up in the player's quality menu, and become the `og:video` target for embeds when the original is not playable. You can queue or remove renditions per clip from its page, or in bulk from **Admin → Files**.

### Encoders

With `ENCODER=auto` the server runs a one-second test encode through each candidate at startup and keeps the first that works, in this order: NVENC, QSV, VAAPI, VideoToolbox, then software x264. The result is on the **Transcoding** page and in `fthr-clips-server check`. Change the setting and detection runs again.

The Docker image carries software x264 only. For a GPU encoder, run the binary with your distribution's `ffmpeg`, or mount a GPU-enabled `ffmpeg` into the container and point `FFMPEG_PATH` at it.

### What to expect on small machines

Posters and probes cost almost nothing. A software 720p transcode of a two-minute clip takes roughly real time on a modern x86 core and several times that on an ARM board. `compat` mode keeps that work to the files that need it; `on` mode is for machines with headroom.

## Tags, search and sort

Tags are free-form labels with a colour. Add them from a clip's page, in bulk from the Files page, or manage the whole set under **Admin → Tags**. The library's chips filter by tag; search matches titles, descriptions and tag names. Sort by newest, oldest, most viewed, title or duration.

## Feeds and notifications

- `/feed.rss` lists the fifty newest public clips and images with enclosures and thumbnails, ready for any reader or automation.
- Set a Discord webhook and a short embed with the poster is posted the moment a clip first becomes public. A generic webhook receives the same event as JSON. Each clip is announced once.
