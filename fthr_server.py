from __future__ import annotations

from typing import Any

import html
import json
import mimetypes
import os
import secrets
import time
from datetime import datetime, timezone
from http.cookies import SimpleCookie
from pathlib import Path
import re

from fastapi import FastAPI, File, Header, HTTPException, Request, Response, UploadFile
from fastapi.responses import FileResponse, HTMLResponse


ADMIN_LOGIN_HTML = """<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>FTHR Clips · Admin login</title><style>body{background:#090a0c;color:#f5f7f8;font:16px system-ui;display:grid;place-items:center;min-height:100vh}main{width:min(380px,calc(100% - 40px));border:1px solid #242a31;border-radius:16px;padding:30px;background:#111419}input,button{width:100%;padding:12px;border-radius:8px;margin-top:12px}input{background:#090a0c;border:1px solid #242a31;color:white}button{background:#c9f36b;border:0;cursor:pointer}</style></head><body><main><p>FTHR CLIPS / ADMIN</p><h1>Sign in</h1><form id="f"><input id="p" type="password" placeholder="Admin password" autofocus><button>Continue</button></form><p id="e"></p><script>f.onsubmit=async e=>{e.preventDefault();let r=await fetch('/admin/login',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({password:p.value})});if(r.ok){location='/admin/dashboard'}else{document.querySelector('#e').textContent='Invalid password'}}</script></main></body></html>"""

ADMIN_HTML = """<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>FTHR Clips · Admin</title><style>body{background:#090a0c;color:#f5f7f8;font:16px system-ui;margin:0;padding:40px}main{max-width:900px;margin:auto}h1{font-size:42px}button{background:#c9f36b;border:0;padding:10px 14px;border-radius:8px;cursor:pointer}.clip{border:1px solid #242a31;padding:14px;margin:10px 0;border-radius:10px;display:flex;justify-content:space-between;gap:12px;align-items:center}.muted{color:#8b949e}</style></head><body><main><p>FTHR CLIPS / ADMIN</p><h1>Manage your clips</h1><p class="muted">Visibility and deletion controls for your library.</p><div id="clips">Loading…</div><script>let csrf='';const esc=s=>String(s).replace(/[&<>\"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;',"'":'&#039;'}[c]));async function load(){const r=await fetch('/admin/api/clips');if(!r.ok){location='/admin';return}const d=await r.json(),root=document.querySelector('#clips');root.replaceChildren();d.clips.forEach(c=>{const row=document.createElement('div');row.className='clip';const info=document.createElement('span');info.textContent=`${c.name} · ${c.size}`;const actions=document.createElement('span');const toggle=document.createElement('button');toggle.textContent=c.listed?'Unlist':'List';toggle.onclick=()=>setListed(c.id,c.listed);const del=document.createElement('button');del.textContent='Delete';del.onclick=()=>delClip(c.id);actions.append(toggle,' ',del);row.append(info,actions);root.append(row)})}async function setListed(id,current){await fetch('/admin/api/clips/'+encodeURIComponent(id),{method:'PATCH',headers:{'Content-Type':'application/json','X-CSRF-Token':csrf},body:JSON.stringify({listed:!current})});load()}async function delClip(id){if(confirm('Delete this clip permanently?')){await fetch('/admin/api/clips/'+encodeURIComponent(id),{method:'DELETE',headers:{'X-CSRF-Token':csrf}});load()}}fetch('/admin/api/session').then(r=>r.ok?r.json():Promise.reject()).then(()=>fetch('/admin/api/csrf')).then(r=>r.json()).then(d=>{csrf=d.csrf;load()}).catch(()=>location='/admin');</script></main></body></html>"""

GALLERY_HTML = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>FTHR Clips · Library</title>
<style>
@import url('https://fonts.googleapis.com/css2?family=DM+Mono:wght@400;500&family=Manrope:wght@400;500;600;700;800&display=swap');
:root{--bg:#0b0d0c;--surface:#121614;--surface-2:#171c19;--line:#27302a;--ink:#f4f7ef;--muted:#98a29a;--lime:#d4f46d;--lime-ink:#10150d;--max:1240px}*{box-sizing:border-box}html{background:var(--bg)}body{margin:0;min-height:100vh;background:linear-gradient(115deg,#0b0d0c 0%,#0b0e0c 60%,#132018 100%);color:var(--ink);font-family:Manrope,system-ui,sans-serif}.shell{width:min(var(--max),calc(100% - 48px));margin:auto;padding:28px 0 72px}.nav{height:54px;display:flex;align-items:center;justify-content:space-between;border-bottom:1px solid var(--line)}.brand{display:flex;align-items:center;gap:11px;font-size:15px;font-weight:800;letter-spacing:.12em}.mark{width:27px;height:27px;display:grid;place-items:center;border-radius:8px;background:var(--lime);color:var(--lime-ink);font-size:14px;font-weight:800}.nav-right{display:flex;align-items:center;gap:10px}.status{display:flex;align-items:center;gap:8px;color:var(--muted);font:11px 'DM Mono',monospace;text-transform:uppercase;letter-spacing:.08em}.dot{width:7px;height:7px;border-radius:50%;background:var(--lime);box-shadow:0 0 0 4px #d4f46d18}.admin{color:var(--muted);text-decoration:none;font-size:12px;padding:9px 12px;border:1px solid var(--line);border-radius:7px}.admin:hover{color:var(--ink);border-color:#56665a}.intro{display:flex;justify-content:space-between;align-items:end;gap:28px;padding:76px 0 48px}.kicker{color:var(--lime);font:11px 'DM Mono',monospace;letter-spacing:.16em;text-transform:uppercase;margin-bottom:17px}.intro h1{font-size:clamp(38px,5.8vw,76px);line-height:.98;letter-spacing:-.075em;margin:0;max-width:720px}.intro h1 em{font-style:normal;color:#8f9e90}.intro-copy{max-width:300px;color:var(--muted);font-size:14px;line-height:1.7;margin:0 0 4px}.toolbar{display:flex;justify-content:space-between;align-items:center;gap:16px;border-top:1px solid var(--line);border-bottom:1px solid var(--line);padding:15px 0;margin-bottom:28px}.library-label{font-size:13px;font-weight:700}.library-label span{color:var(--muted);font-weight:500;margin-left:8px}.tools{display:flex;align-items:center;gap:9px}.sort{font:11px 'DM Mono',monospace;color:var(--muted);border:1px solid var(--line);padding:9px 11px;border-radius:7px}.refresh{height:34px;background:transparent;color:var(--ink);border:1px solid var(--line);border-radius:7px;padding:0 12px;cursor:pointer;font:12px Manrope}.refresh:hover{background:var(--surface-2);border-color:#56665a}.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:26px 18px}.card{min-width:0}.preview{aspect-ratio:16/9;position:relative;overflow:hidden;border-radius:10px;background:#171c19;border:1px solid var(--line);box-shadow:0 12px 28px #00000018}.preview:after{content:'';position:absolute;inset:0;background:linear-gradient(180deg,#00000000 55%,#00000068);pointer-events:none}.preview video{width:100%;height:100%;display:block;object-fit:cover;transition:transform .45s ease}.card:hover .preview video{transform:scale(1.035)}.file-art{height:100%;display:grid;place-items:center;color:var(--lime);font:24px 'DM Mono',monospace}.tag{position:absolute;z-index:1;top:11px;left:11px;padding:5px 7px;border-radius:5px;background:#0b0d0ccc;color:#dce3d5;font:10px 'DM Mono',monospace;letter-spacing:.08em}.play{position:absolute;z-index:1;left:50%;top:50%;transform:translate(-50%,-50%);width:43px;height:43px;border-radius:50%;display:grid;place-items:center;background:#d4f46de8;color:#10150d;opacity:0;transition:opacity .2s,transform .2s;font-size:15px;padding-left:2px}.card:hover .play{opacity:1;transform:translate(-50%,-50%) scale(1.04)}.card-body{padding:13px 2px 0}.name{font-size:14px;font-weight:700;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;color:var(--ink)}.meta{display:flex;gap:8px;align-items:center;color:var(--muted);font:11px 'DM Mono',monospace;margin-top:7px}.meta i{width:3px;height:3px;background:#566158;border-radius:50%}.actions{display:flex;gap:8px;margin-top:13px}.actions a,.actions button{font:12px Manrope;font-weight:700;text-decoration:none;border-radius:6px;padding:8px 11px;cursor:pointer}.actions a{background:var(--lime);color:var(--lime-ink)}.actions button{background:transparent;color:var(--muted);border:1px solid var(--line)}.actions button:hover{color:var(--ink);border-color:#56665a}.empty{grid-column:1/-1;border:1px dashed #354239;border-radius:10px;padding:72px 20px;text-align:center;color:var(--muted)}.empty b{display:block;color:var(--ink);font-size:18px;margin-bottom:8px}.loading{color:var(--muted);font:12px 'DM Mono',monospace;padding:42px 0}@media(max-width:800px){.grid{grid-template-columns:repeat(2,1fr)}.intro{padding:60px 0 38px}.intro-copy{display:none}}@media(max-width:560px){.shell{width:min(100% - 30px,1240px);padding-top:16px}.nav{height:48px}.status{display:none}.intro{padding:48px 0 34px}.intro h1{font-size:48px}.toolbar{align-items:flex-start}.sort{display:none}.grid{grid-template-columns:1fr;gap:24px}.preview{border-radius:9px}.play{opacity:1}.actions a,.actions button{min-height:38px}}
</style></head><body><div class="shell"><header class="nav"><div class="brand"><span class="mark">F</span><span>FTHR / CLIPS</span></div><div class="nav-right"><div class="status"><span class="dot"></span><span>Library online</span></div><a class="admin" href="/admin">Manage</a></div></header><section class="intro"><div><div class="kicker">Personal capture library</div><h1>Moments worth<br><em>keeping close.</em></h1></div><p class="intro-copy">A quiet place for the things you capture. Browse the latest clips, open one, or share it with a single link.</p></section><div class="toolbar"><div class="library-label">Latest captures <span id="count">— clips</span></div><div class="tools"><span class="sort">Newest first</span><button class="refresh" onclick="load()">Refresh</button></div></div><section id="grid" class="grid"><div class="loading">Loading library…</div></section></div><script>
const grid=document.querySelector('#grid'),count=document.querySelector('#count');
function esc(s){return String(s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#039;'}[c]))}
function card(c){const video=c.content_type?.startsWith('video/');const safe=esc(c.url);return `<article class="card"><div class="preview">${video?`<video src="${safe}" preload="metadata" muted playsinline></video><span class="play">▶</span>`:`<div class="file-art">◈</div>`}<span class="tag">${esc(c.extension||'CLIP')}</span></div><div class="card-body"><div class="name" title="${esc(c.name)}">${esc(c.name)}</div><div class="meta"><span>${esc(c.size)}</span><i></i><span>${esc(c.created)}</span></div><div class="actions"><a href="${safe}" target="_blank" rel="noopener">Open clip</a><button data-url="${safe}">Copy link</button></div></div></article>`}
async function load(){grid.innerHTML='<div class="loading">Loading library…</div>';try{const r=await fetch('/api/clips');if(!r.ok)throw Error();const d=await r.json();count.textContent=`${d.clips.length} ${d.clips.length===1?'clip':'clips'}`;grid.innerHTML=d.clips.length?d.clips.map(card).join(''):'<div class="empty"><b>No clips yet</b>Your next great moment will appear here.</div>';grid.querySelectorAll('button[data-url]').forEach(b=>b.onclick=async()=>{try{await navigator.clipboard.writeText(location.origin+b.dataset.url);b.textContent='Copied';setTimeout(()=>b.textContent='Copy link',1400)}catch{b.textContent='Copy failed'}})}catch(e){grid.innerHTML='<div class="empty"><b>Library unavailable</b>Try refreshing in a moment.</div>'}}load();
</script></body></html>"""


def _format_size(size: int) -> str:
    if size < 1024 * 1024:
        return f"{size / 1024:.0f} KB"
    return f"{size / (1024 * 1024):.1f} MB"


def create_app(storage_dir: str | Path = "/data", token: str | None = None, public_base_url: str = "", max_upload_bytes: int = 524_288_000, admin_password: str | None = None, upload_limit: int = 30) -> FastAPI:
    storage = Path(storage_dir)
    storage.mkdir(parents=True, exist_ok=True)
    expected_token = token if token is not None else os.environ.get("UPLOAD_TOKEN", "")
    if not expected_token:
        raise ValueError("UPLOAD_TOKEN is required")
    app = FastAPI(title="FTHR Clips Server", docs_url=None, redoc_url=None)
    expected_admin = admin_password if admin_password is not None else os.environ.get("ADMIN_PASSWORD", "")
    sessions: dict[str, tuple[str, float]] = {}
    upload_timestamps: list[float] = []
    login_timestamps: list[float] = []

    def authorize(authorization: str | None) -> None:
        if not authorization or not secrets.compare_digest(authorization, f"Bearer {expected_token}"):
            raise HTTPException(status_code=401, detail="Unauthorized")

    def admin_session(cookie: str | None) -> tuple[str, str]:
        session = sessions.get(cookie or "")
        if not session or session[1] < time.time():
            raise HTTPException(status_code=401, detail="Admin login required")
        return cookie or "", session[0]

    def require_csrf(request_csrf: str | None, expected: str) -> None:
        if not request_csrf or not secrets.compare_digest(request_csrf, expected):
            raise HTTPException(status_code=403, detail="CSRF check failed")

    @app.post("/admin/login")
    def admin_login(payload: dict[str, str], response: Response) -> dict[str, str]:
        now = time.time()
        login_timestamps[:] = [stamp for stamp in login_timestamps if stamp > now - 60]
        if len(login_timestamps) >= 10:
            raise HTTPException(status_code=429, detail="Login rate limit exceeded")
        login_timestamps.append(now)
        if not expected_admin or not secrets.compare_digest(payload.get("password", ""), expected_admin):
            raise HTTPException(status_code=401, detail="Invalid admin password")
        session_id, csrf = secrets.token_urlsafe(32), secrets.token_urlsafe(24)
        sessions[session_id] = (csrf, time.time() + 8 * 3600)
        response.set_cookie("fthr_admin", session_id, httponly=True, secure=False, samesite="strict", max_age=8 * 3600)
        return {"csrf": csrf}

    @app.post("/admin/logout")
    def admin_logout(request_cookie: str | None = Header(default=None, alias="Cookie")) -> dict[str, str]:
        session_id = cookie_session(request_cookie)
        if session_id:
            sessions.pop(session_id, None)
        return {"status": "ok"}

    @app.get("/admin", response_class=HTMLResponse)
    def admin_login_page() -> str:
        return ADMIN_LOGIN_HTML

    @app.get("/admin/dashboard", response_class=HTMLResponse)
    def admin_dashboard(request_cookie: str | None = Header(default=None, alias="Cookie")) -> str:
        admin_session(cookie_session(request_cookie))
        return ADMIN_HTML

    def cookie_session(request_cookie: str | None) -> str | None:
        cookies = SimpleCookie(request_cookie or "")
        return cookies.get("fthr_admin").value if cookies.get("fthr_admin") else None

    @app.get("/admin/api/session")
    def admin_session_check(request_cookie: str | None = Header(default=None, alias="Cookie")) -> dict[str, bool]:
        admin_session(cookie_session(request_cookie))
        return {"authenticated": True}

    @app.get("/admin/api/csrf")
    def admin_csrf(request_cookie: str | None = Header(default=None, alias="Cookie")) -> dict[str, str]:
        _, csrf = admin_session(cookie_session(request_cookie))
        return {"csrf": csrf}

    @app.get("/admin/api/clips")
    def admin_clips(request_cookie: str | None = Header(default=None, alias="Cookie")) -> dict[str, list[dict[str, Any]]]:
        admin_session(cookie_session(request_cookie))
        return list_clips(include_unlisted=True)

    @app.patch("/admin/api/clips/{identifier}")
    def update_clip(identifier: str, payload: dict[str, bool], request_cookie: str | None = Header(default=None, alias="Cookie"), csrf: str | None = Header(default=None, alias="X-CSRF-Token")) -> dict[str, str]:
        session_id = cookie_session(request_cookie)
        _, expected_csrf = admin_session(session_id); require_csrf(csrf, expected_csrf)
        metadata_path = storage / f"{identifier}.json"
        if not metadata_path.exists(): raise HTTPException(status_code=404, detail="Not found")
        metadata = json.loads(metadata_path.read_text()); metadata["listed"] = bool(payload.get("listed", True)); metadata_path.write_text(json.dumps(metadata))
        return {"status": "ok"}

    @app.delete("/admin/api/clips/{identifier}")
    def delete_clip(identifier: str, request_cookie: str | None = Header(default=None, alias="Cookie"), csrf: str | None = Header(default=None, alias="X-CSRF-Token")) -> dict[str, str]:
        session_id = cookie_session(request_cookie)
        _, expected_csrf = admin_session(session_id); require_csrf(csrf, expected_csrf)
        (storage / identifier).unlink(missing_ok=True); (storage / f"{identifier}.json").unlink(missing_ok=True)
        return {"status": "deleted"}

    @app.middleware("http")
    async def security_headers(request: Request, call_next):
        response = await call_next(request)
        response.headers["X-Content-Type-Options"] = "nosniff"
        response.headers["X-Frame-Options"] = "DENY"
        response.headers["Referrer-Policy"] = "no-referrer"
        response.headers["Content-Security-Policy"] = "default-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; media-src 'self'; script-src 'self' 'unsafe-inline'"
        if request.url.scheme == "https":
            response.headers["Strict-Transport-Security"] = "max-age=31536000; includeSubDomains"
        return response

    @app.get("/", response_class=HTMLResponse)
    def gallery() -> str:
        return GALLERY_HTML

    @app.get("/healthz")
    def healthz() -> dict[str, str]:
        return {"status": "ok"}

    def list_clips(include_unlisted: bool = False) -> dict[str, list[dict[str, Any]]]:
        base = public_base_url.rstrip("/")
        items = []
        for target in storage.iterdir():
            if not target.is_file() or target.name.endswith(".json"):
                continue
            metadata_path = target.with_name(f"{target.name}.json")
            metadata = json.loads(metadata_path.read_text()) if metadata_path.exists() else {}
            if not include_unlisted and metadata.get("listed", True) is False:
                continue
            content_type = metadata.get("content_type") or mimetypes.guess_type(metadata.get("name", ""))[0] or "application/octet-stream"
            created = datetime.fromtimestamp(target.stat().st_mtime, timezone.utc).strftime("%d %b %Y")
            items.append({"id": target.name, "name": metadata.get("name", target.name), "url": f"{base}/files/{target.name}", "size": _format_size(target.stat().st_size), "created": created, "extension": Path(metadata.get("name", target.name)).suffix.lstrip(".").upper() or "FILE", "content_type": content_type, "listed": bool(metadata.get("listed", True))})
        items.sort(key=lambda item: item["created"], reverse=True)
        return {"clips": items}

    @app.get("/api/clips")
    def clips() -> dict[str, list[dict[str, Any]]]:
        return list_clips()

    @app.head("/upload")
    def upload_probe(authorization: str | None = Header(default=None)) -> None:
        authorize(authorization)

    @app.post("/upload")
    async def upload(clip: UploadFile = File(...), authorization: str | None = Header(default=None)) -> dict[str, str]:
        authorize(authorization)
        now = time.time()
        upload_timestamps[:] = [stamp for stamp in upload_timestamps if stamp > now - 60]
        if len(upload_timestamps) >= upload_limit:
            raise HTTPException(status_code=429, detail="Upload rate limit exceeded")
        upload_timestamps.append(now)
        if not clip.filename:
            raise HTTPException(status_code=400, detail="A clip filename is required")
        identifier = secrets.token_urlsafe(18)
        target = storage / identifier
        size = 0
        try:
            with target.open("xb") as output:
                while chunk := await clip.read(1024 * 1024):
                    size += len(chunk)
                    if size > max_upload_bytes:
                        raise HTTPException(status_code=413, detail="Upload is too large")
                    output.write(chunk)
            target.with_name(f"{identifier}.json").write_text(json.dumps({"name": Path(clip.filename).name, "content_type": clip.content_type or "application/octet-stream", "listed": True}))
        except HTTPException:
            target.unlink(missing_ok=True)
            raise
        except OSError as exc:
            target.unlink(missing_ok=True)
            raise HTTPException(status_code=500, detail="Could not store upload") from exc
        base = public_base_url.rstrip("/")
        return {"url": f"{base}/files/{identifier}", "id": identifier, "bytes": str(size), "listed": "true"}

    @app.get("/files/{identifier}")
    def download(identifier: str) -> FileResponse:
        if not re.fullmatch(r"[A-Za-z0-9_-]{20,32}", identifier):
            raise HTTPException(status_code=404, detail="Not found")
        target = storage / identifier
        if target.is_symlink() or not target.is_file() or target.resolve().parent != storage.resolve():
            raise HTTPException(status_code=404, detail="Not found")
        metadata_path = target.with_name(f"{identifier}.json")
        metadata = json.loads(metadata_path.read_text()) if metadata_path.exists() else {}
        return FileResponse(target, media_type=metadata.get("content_type", "application/octet-stream"), filename=metadata.get("name", identifier))

    return app


app = create_app(storage_dir=os.environ.get("STORAGE_DIR", "/data"), public_base_url=os.environ.get("PUBLIC_BASE_URL", ""), max_upload_bytes=int(os.environ.get("MAX_UPLOAD_BYTES", "524288000")))

if __name__ == "__main__":
    import uvicorn
    uvicorn.run("fthr_server:app", host=os.environ.get("BIND_HOST", "127.0.0.1"), port=int(os.environ.get("PORT", "8080")))
