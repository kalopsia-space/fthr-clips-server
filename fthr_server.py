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
<title>FTHR Clips · Your library</title>
<style>
@import url('https://fonts.googleapis.com/css2?family=DM+Mono:wght@400;500&family=Space+Grotesk:wght@400;500;600;700&display=swap');
:root{--bg:#090a0c;--panel:#111419;--line:#242a31;--text:#f5f7f8;--muted:#8b949e;--lime:#c9f36b;--pink:#ff7b9c}*{box-sizing:border-box}body{margin:0;background:radial-gradient(circle at 80% -10%,#243326 0,#090a0c 35%);color:var(--text);font-family:'Space Grotesk',system-ui,sans-serif;min-height:100vh}main{width:min(1180px,calc(100% - 40px));margin:auto;padding:38px 0 70px}.top{display:flex;justify-content:space-between;align-items:center;gap:20px}.brand{display:flex;align-items:center;gap:12px;font-weight:700;letter-spacing:-.04em;font-size:20px}.mark{display:grid;place-items:center;width:38px;height:38px;border-radius:12px;background:var(--lime);color:#111;font-weight:700}.pill{font:12px 'DM Mono',monospace;color:var(--muted);border:1px solid var(--line);border-radius:999px;padding:9px 13px}.hero{padding:84px 0 52px;max-width:680px}.eyebrow{color:var(--lime);font:12px 'DM Mono',monospace;letter-spacing:.14em;text-transform:uppercase}.hero h1{font-size:clamp(42px,7vw,80px);line-height:.95;letter-spacing:-.075em;margin:16px 0 20px}.hero p{color:var(--muted);font-size:18px;line-height:1.55;margin:0}.bar{border-top:1px solid var(--line);border-bottom:1px solid var(--line);padding:16px 0;display:flex;justify-content:space-between;align-items:center;margin-bottom:24px}.bar strong{font-size:14px}.refresh{background:none;border:1px solid var(--line);color:var(--text);border-radius:8px;padding:9px 13px;cursor:pointer}.refresh:hover{border-color:var(--lime);color:var(--lime)}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:18px}.card{background:linear-gradient(145deg,#171b20,#0f1115);border:1px solid var(--line);border-radius:16px;overflow:hidden;transition:transform .2s,border-color .2s}.card:hover{transform:translateY(-3px);border-color:#718047}.preview{height:160px;background:#1b2025;position:relative;display:grid;place-items:center;color:var(--lime);font:42px 'DM Mono',monospace}.preview video{width:100%;height:100%;object-fit:cover}.tag{position:absolute;top:12px;left:12px;background:#090a0ccc;border:1px solid #ffffff1a;border-radius:6px;padding:5px 7px;font:10px 'DM Mono',monospace;color:var(--lime)}.body{padding:16px}.name{white-space:nowrap;overflow:hidden;text-overflow:ellipsis;font-weight:600}.meta{color:var(--muted);font:11px 'DM Mono',monospace;margin-top:7px}.actions{display:flex;gap:8px;margin-top:16px}.actions a,.actions button{flex:1;text-align:center;text-decoration:none;border:1px solid var(--line);border-radius:8px;padding:9px;background:transparent;color:var(--text);font:12px 'DM Mono',monospace;cursor:pointer}.actions a:first-child{background:var(--lime);color:#10130b;border-color:var(--lime);font-weight:500}.empty{text-align:center;border:1px dashed var(--line);border-radius:16px;padding:64px 20px;color:var(--muted);grid-column:1/-1}.empty b{display:block;color:var(--text);font-size:20px;margin-bottom:8px}@media(max-width:600px){main{width:min(100% - 28px,1180px);padding-top:22px}.hero{padding:65px 0 40px}.pill{display:none}}
</style></head><body><main><header class="top"><div class="brand"><span class="mark">F</span> FTHR CLIPS</div><div class="pill">PRIVATE LIBRARY · <span id="count">—</span> CLIPS</div></header><section class="hero"><div class="eyebrow">Your moments, on your terms</div><h1>Play it back.</h1><p>A calm, fast home for the clips you capture. Share a link, relive the moment, keep the archive yours.</p></section><div class="bar"><strong>Latest captures</strong><button class="refresh" onclick="load()">↻ Refresh</button></div><section id="grid" class="grid"><div class="empty">Loading your library…</div></section></main><script>
const grid=document.querySelector('#grid'), count=document.querySelector('#count');
function esc(s){return String(s).replace(/[&<>\"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;',"'":'&#039;'}[c]))}
async function load(){try{const r=await fetch('/api/clips');if(!r.ok)throw Error();const d=await r.json();count.textContent=d.clips.length;grid.innerHTML=d.clips.length?d.clips.map(c=>{const video=c.content_type?.startsWith('video/');return `<article class="card"><div class="preview">${video?`<video src="${esc(c.url)}" preload="metadata"></video>`:'◈'}<span class="tag">${esc(c.extension||'CLIP')}</span></div><div class="body"><div class="name" title="${esc(c.name)}">${esc(c.name)}</div><div class="meta">${esc(c.size)} · ${esc(c.created)}</div><div class="actions"><a href="${esc(c.url)}" target="_blank">Open</a><button onclick="navigator.clipboard.writeText(location.origin+'${esc(c.url)}');this.textContent='Copied!'">Copy link</button></div></div></article>`}).join(''):'<div class="empty"><b>No clips yet</b>Your next great moment will show up here.</div>'}catch(e){grid.innerHTML='<div class="empty"><b>Library unavailable</b>Try refreshing in a moment.</div>'}}
load();
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
