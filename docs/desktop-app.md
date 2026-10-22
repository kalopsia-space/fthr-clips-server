# Desktop app setup

FTHR Clips has a "Your server" upload provider. This server is what it was built for.

## Connecting FTHR Clips

1. In the app, open the upload settings and pick **Your server**.
2. Fill in two fields:

   | Field | Value |
   |---|---|
   | URL | `https://clips.example.com/upload` (your public address plus `/upload`) |
   | Authorization | `Bearer ` followed by your upload token |

3. Press **Test connection**. The app sends a `HEAD` request; a green result means the URL and token are right.
4. Share a clip. The app uploads it, the server answers with the clip's page URL, and the app copies that to your clipboard.

The share link is public or link-only according to **Admin → Library → uploads start public**. Either way it works in Discord straight away.

## Upload tokens

The token in `UPLOAD_TOKEN` always works. If you share the server with friends, give each person their own token from **Admin → Tokens**: tokens can be named, show when they were last used, and can be revoked one at a time. Tokens are shown once at creation and stored hashed.

## The upload contract

Anything that can make an HTTP request can upload. This is the whole protocol:

```
POST /upload
Authorization: Bearer <token>
Content-Type: multipart/form-data

clip=<the file, with its original filename>
```

| Response | Meaning |
|---|---|
| `200` with `{"url": "...", "id": "...", "bytes": N, "listed": true}` | Done. `url` is the share link. |
| `401` | Missing or wrong token. |
| `413` | Larger than `MAX_UPLOAD_BYTES`. The partial file is discarded. |
| `415` | Not a video or image the server recognises. |
| `429` | Rate limited. Thirty uploads a minute per token; twenty failed logins a minute per address. |

`HEAD /upload` with the token answers `200` and is what the app's connection test uses.

Uploading a file whose content is already in the library returns the existing clip's URL rather than a duplicate. Accepted types are MP4, M4V, MOV, WebM, MKV and AVI for video, and PNG, JPEG, WebP and GIF for images.

A `curl` upload for scripts:

```sh
curl -H "Authorization: Bearer $TOKEN" -F "clip=@ace.mp4" https://clips.example.com/upload
```
