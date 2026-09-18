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
<title>FTHR Clips</title>
<style>
@import url('https://fonts.googleapis.com/css2?family=DM+Mono:wght@400;500&family=Inter:wght@400;500;600;700&display=swap');
:root{--bg:#10131b;--sidebar:#171b26;--panel:#1b202c;--panel2:#222938;--line:#2b3343;--text:#f6f7fb;--muted:#9099ab;--blue:#62a7ff;--blue2:#397fe1;--pink:#e45a9b;--danger:#ff8d9b}*{box-sizing:border-box}html,body{margin:0;background:var(--bg);color:var(--text);font-family:Inter,system-ui,sans-serif}body{min-height:100vh}.app{min-height:100vh;display:flex}.sidebar{position:fixed;inset:0 auto 0 0;width:248px;background:var(--sidebar);border-right:1px solid #252c3a;padding:25px 17px 18px;display:flex;flex-direction:column;z-index:5}.brand{display:flex;align-items:center;gap:11px;padding:0 10px 31px;font-size:20px;font-weight:700;letter-spacing:-.04em}.logo{width:31px;height:31px;border-radius:9px;background:linear-gradient(135deg,#62a7ff,#df5f9f);display:grid;place-items:center;color:#fff;font-weight:800}.nav{display:grid;gap:5px}.nav a{display:flex;align-items:center;gap:13px;color:#aeb7c8;text-decoration:none;padding:12px 13px;border-radius:9px;font-size:13px;font-weight:600}.nav a:hover{background:#232a39;color:#fff}.nav a.active{color:#fff;background:linear-gradient(105deg,#d255a2,#9c57d4);box-shadow:0 8px 20px #ae4eb544}.ico{width:18px;height:18px;display:grid;place-items:center;color:currentColor}.side-rule{height:1px;background:#2b3341;margin:24px 10px 18px}.upload{border:1px dashed #46536b;border-radius:10px;padding:18px 13px;text-align:center;color:#9ca7bb;font-size:11px;line-height:1.5}.upload .upload-icon{color:var(--blue);font-size:22px;display:block;margin-bottom:8px}.upload strong{color:#eef2f9;display:block;font-size:12px;margin-bottom:4px}.upload a{display:inline-block;margin-top:11px;color:#fff;background:#2b78d2;text-decoration:none;padding:7px 11px;border-radius:6px;font-size:11px}.side-bottom{margin-top:auto;color:#788397;font:10px 'DM Mono',monospace;padding:17px 10px 0;border-top:1px solid #2b3341}.main{margin-left:248px;min-width:0;flex:1;padding:0 30px 55px}.topbar{height:83px;display:flex;align-items:center;gap:18px;max-width:1600px;margin:auto}.mobile-brand{display:none}.search{height:42px;flex:1;max-width:590px;position:relative}.search svg{position:absolute;left:14px;top:12px;color:#8490a5}.search input{width:100%;height:100%;background:var(--panel);border:1px solid var(--line);border-radius:8px;color:var(--text);padding:0 15px 0 43px;outline:none;font:13px Inter}.search input::placeholder{color:#768096}.search input:focus{border-color:#4f86c9;box-shadow:0 0 0 3px #4e91d822}.top-spacer{flex:1}.sort{height:40px;background:var(--panel);border:1px solid var(--line);border-radius:7px;color:#b9c1d0;padding:0 12px;font:12px Inter;outline:none}.manage{height:40px;padding:0 13px;border-radius:7px;border:1px solid #3c75b6;background:#1d4978;color:#eaf4ff;text-decoration:none;display:flex;align-items:center;font-size:12px}.content{max-width:1600px;margin:auto}.heading{display:flex;justify-content:space-between;align-items:end;border-bottom:1px solid #293242;padding:16px 0 17px;margin-bottom:24px}.heading h1{font-size:26px;margin:0;letter-spacing:-.04em}.heading p{margin:7px 0 0;color:var(--muted);font-size:12px}.count{color:#8792a5;font:11px 'DM Mono',monospace}.grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:27px 18px}.card{min-width:0;background:transparent;cursor:pointer}.thumb{position:relative;aspect-ratio:16/9;overflow:hidden;background:#202737;border-radius:8px;border:1px solid #30394b}.thumb video{width:100%;height:100%;display:block;object-fit:cover;transition:transform .25s}.card:hover .thumb video{transform:scale(1.025)}.thumb:after{content:'';position:absolute;inset:0;background:linear-gradient(180deg,transparent 58%,#05070ccc);pointer-events:none}.file-thumb{height:100%;display:grid;place-items:center;font:20px 'DM Mono',monospace;color:var(--blue);background:radial-gradient(circle at 50% 25%,#2a4666,#202737 58%)}.badge,.duration{position:absolute;z-index:1;bottom:9px;background:#080b12c9;color:#f3f5fa;border-radius:4px;padding:4px 6px;font:10px 'DM Mono',monospace}.badge{left:9px}.duration{right:9px}.type{position:absolute;z-index:1;top:9px;left:9px;border-radius:4px;background:#111725d9;color:#bfcbea;padding:4px 6px;font:9px 'DM Mono',monospace;letter-spacing:.08em}.card-info{display:flex;gap:10px;padding:11px 2px 0}.avatar{flex:0 0 27px;width:27px;height:27px;border-radius:50%;background:linear-gradient(135deg,#62a7ff,#c052a0);display:grid;place-items:center;font-size:11px;font-weight:700}.info{min-width:0;flex:1}.name{font-size:13px;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;color:#f1f3f8}.meta{font:10px 'DM Mono',monospace;color:#818ca0;margin-top:6px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.more{border:0;background:none;color:#8d98ab;font-size:18px;padding:0 4px;align-self:flex-start;cursor:pointer}.more:hover{color:#fff}.empty,.loading{grid-column:1/-1;text-align:center;padding:90px 20px;border:1px dashed #39465b;border-radius:10px;color:var(--muted)}.empty strong{display:block;color:#fff;margin-bottom:8px}.skeleton{height:230px;border-radius:8px;background:linear-gradient(90deg,#1a202c 25%,#242c3b 37%,#1a202c 63%);background-size:400% 100%;animation:shimmer 1.4s infinite}@keyframes shimmer{0%{background-position:100% 0}100%{background-position:-100% 0}}.modal{display:none;position:fixed;inset:0;z-index:20;background:#05070bd9;align-items:center;justify-content:center;padding:25px}.modal.open{display:flex}.modal-box{width:min(1000px,100%);background:#171c27;border:1px solid #354056;border-radius:11px;box-shadow:0 20px 70px #000a;overflow:hidden}.modal-head{display:flex;justify-content:space-between;align-items:center;padding:13px 16px;border-bottom:1px solid #2b3444}.modal-title{font-size:13px;font-weight:600;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.close{border:0;background:none;color:#9aa6ba;font-size:24px;cursor:pointer}.modal video{display:block;width:100%;max-height:75vh;background:#07090d}.modal-actions{display:flex;justify-content:flex-end;gap:8px;padding:12px 16px}.modal-actions a,.modal-actions button{border:1px solid #3b4659;border-radius:6px;background:#202838;color:#eaf0fa;padding:8px 12px;text-decoration:none;font:12px Inter;cursor:pointer}.modal-actions a{background:#2b78d2;border-color:#438bd8}@media(max-width:1250px){.grid{grid-template-columns:repeat(3,minmax(0,1fr))}}@media(max-width:860px){.sidebar{width:215px}.main{margin-left:215px;padding:0 20px}.grid{grid-template-columns:repeat(2,minmax(0,1fr))}.top-spacer{display:none}.manage{display:none}}@media(max-width:600px){.sidebar{display:none}.main{margin-left:0;padding:0 14px 35px}.topbar{height:70px;gap:10px}.mobile-brand{display:flex;align-items:center;gap:7px;font-size:14px;font-weight:700;margin-right:5px}.mobile-brand .logo{width:26px;height:26px;font-size:12px}.search{order:2}.topbar .sort{display:none}.content{width:100%}.heading{padding-top:11px;margin-bottom:17px}.heading h1{font-size:22px}.heading p{font-size:11px}.grid{grid-template-columns:repeat(2,minmax(0,1fr));gap:23px 10px}.card-info{gap:7px;padding-top:8px}.avatar{width:22px;height:22px;flex-basis:22px;font-size:9px}.name{font-size:11px}.meta{font-size:8px;margin-top:4px}.more{font-size:15px}.thumb{border-radius:6px}.type{top:6px;left:6px;font-size:8px}.badge,.duration{bottom:6px;font-size:8px;padding:3px 4px}.modal{padding:10px}}
</style></head><body><div class="app"><aside class="sidebar"><div class="brand"><span class="logo">F</span><span>FTHR Clips</span></div><nav class="nav"><a href="/" class="active"><span class="ico">▦</span>My clips</a><a href="/"><span class="ico">◉</span>Public library</a></nav><div class="side-rule"></div><div class="upload"><span class="upload-icon">↥</span><strong>Upload a clip</strong>Use FTHR Clips to send a new recording here.<a href="/admin">Manage uploads</a></div><div class="side-bottom">FTHR CLIPS / PRIVATE LIBRARY<br><br>PUBLIC SHARING ENABLED</div></aside><main class="main"><header class="topbar"><div class="mobile-brand"><span class="logo">F</span>FTHR</div><div class="search"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></svg><input id="search" type="search" placeholder="Search clips…" autocomplete="off"></div><div class="top-spacer"></div><select id="sort" class="sort" aria-label="Sort clips"><option value="newest">Newest first</option><option value="oldest">Oldest first</option><option value="name">Name</option></select><a class="manage" href="/admin">Manage</a></header><section class="content"><div class="heading"><div><h1>My clips</h1><p>Browse and share your latest captures.</p></div><span id="count" class="count">—</span></div><section id="grid" class="grid"><div class="skeleton"></div><div class="skeleton"></div><div class="skeleton"></div><div class="skeleton"></div></section></section></main></div><div id="modal" class="modal" role="dialog" aria-modal="true" aria-label="Clip preview"><div class="modal-box"><div class="modal-head"><div id="modal-title" class="modal-title"></div><button class="close" id="close" aria-label="Close preview">×</button></div><video id="modal-video" controls playsinline></video><div class="modal-actions"><a id="modal-open" target="_blank" rel="noopener">Open direct link</a><button id="modal-copy">Copy link</button></div></div></div><script>
const grid=document.querySelector('#grid'),search=document.querySelector('#search'),sort=document.querySelector('#sort'),count=document.querySelector('#count'),modal=document.querySelector('#modal'),mv=document.querySelector('#modal-video'),mt=document.querySelector('#modal-title'),mo=document.querySelector('#modal-open');let clips=[];
function esc(s){return String(s).replace(/[&<>\"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;',"'":'&#039;'}[c]))}
function render(){let q=search.value.toLowerCase().trim(),items=clips.filter(c=>!q||c.name.toLowerCase().includes(q));items.sort((a,b)=>sort.value==='name'?a.name.localeCompare(b.name):sort.value==='oldest'?a.created.localeCompare(b.created):b.created.localeCompare(a.created));count.textContent=`${items.length} ${items.length===1?'clip':'clips'}`;if(!items.length){grid.innerHTML='<div class="empty"><strong>No clips found</strong>Try a different search or upload a new clip.</div>';return}grid.innerHTML=items.map((c,i)=>{let video=c.content_type?.startsWith('video/');return `<article class="card" data-id="${esc(c.id)}" tabindex="0"><div class="thumb">${video?`<video src="${esc(c.url)}" preload="metadata" muted playsinline></video>`:`<div class="file-thumb">◈</div>`}<span class="type">${esc(c.extension||'FILE')}</span><span class="badge">${video?'▶ VIDEO':'FILE'}</span><span class="duration">${esc(c.size)}</span></div><div class="card-info"><span class="avatar">F</span><div class="info"><div class="name" title="${esc(c.name)}">${esc(c.name)}</div><div class="meta">FTHR CLIPS · ${esc(c.created)}</div></div><button class="more" data-copy="${esc(c.url)}" aria-label="Copy link">⋮</button></div></article>`}).join('');grid.querySelectorAll('.card').forEach(card=>{let c=clips.find(x=>x.id===card.dataset.id);card.onclick=e=>{if(e.target.closest('.more'))return;openModal(c)};card.onkeydown=e=>{if(e.key==='Enter'||e.key===' ')openModal(c)};let v=card.querySelector('video');if(v){card.onmouseenter=()=>v.play().catch(()=>{});card.onmouseleave=()=>{v.pause();v.currentTime=0}}});grid.querySelectorAll('[data-copy]').forEach(b=>b.onclick=async e=>{e.stopPropagation();await navigator.clipboard.writeText(location.origin+b.dataset.copy);b.textContent='✓';setTimeout(()=>b.textContent='⋮',1200)})}
function openModal(c){if(!c)return;mt.textContent=c.name;mv.src=c.url;mo.href=c.url;modal.classList.add('open');mv.play().catch(()=>{})}function closeModal(){mv.pause();mv.removeAttribute('src');modal.classList.remove('open')}document.querySelector('#close').onclick=closeModal;modal.onclick=e=>{if(e.target===modal)closeModal()};document.onkeydown=e=>{if(e.key==='Escape')closeModal()};document.querySelector('#modal-copy').onclick=async()=>{await navigator.clipboard.writeText(location.origin+mv.src.replace(location.origin,''));document.querySelector('#modal-copy').textContent='Copied';setTimeout(()=>document.querySelector('#modal-copy').textContent='Copy link',1300)};search.oninput=render;sort.onchange=render;
async function load(){try{let r=await fetch('/api/clips');if(!r.ok)throw Error();clips=(await r.json()).clips;render()}catch(e){grid.innerHTML='<div class="empty"><strong>Library unavailable</strong>Try refreshing in a moment.</div>'}}load();
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
    def admin_login(payload: dict[str, str], request: Request, response: Response) -> dict[str, str]:
        now = time.time()
        login_timestamps[:] = [stamp for stamp in login_timestamps if stamp > now - 60]
        if len(login_timestamps) >= 10:
            raise HTTPException(status_code=429, detail="Login rate limit exceeded")
        login_timestamps.append(now)
        if not expected_admin or not secrets.compare_digest(payload.get("password", ""), expected_admin):
            raise HTTPException(status_code=401, detail="Invalid admin password")
        session_id, csrf = secrets.token_urlsafe(32), secrets.token_urlsafe(24)
        sessions[session_id] = (csrf, time.time() + 8 * 3600)
        is_https = request.url.scheme == "https" or request.headers.get("x-forwarded-proto", "").split(",")[0].strip().lower() == "https"
        response.set_cookie("fthr_admin", session_id, httponly=True, secure=is_https, samesite="strict", max_age=8 * 3600)
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
