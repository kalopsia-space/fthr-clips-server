from __future__ import annotations

import os
import secrets
from pathlib import Path

from fastapi import FastAPI, File, Header, HTTPException, UploadFile
from fastapi.responses import FileResponse


def create_app(storage_dir: str | Path = "/data", token: str | None = None, public_base_url: str = "", max_upload_bytes: int = 524_288_000) -> FastAPI:
    storage = Path(storage_dir)
    storage.mkdir(parents=True, exist_ok=True)
    expected_token = token if token is not None else os.environ.get("UPLOAD_TOKEN", "")
    if not expected_token:
        raise ValueError("UPLOAD_TOKEN is required")
    app = FastAPI(title="FTHR Clips Server", docs_url=None, redoc_url=None)

    def authorize(authorization: str | None) -> None:
        if not authorization or not secrets.compare_digest(authorization, f"Bearer {expected_token}"):
            raise HTTPException(status_code=401, detail="Unauthorized")

    @app.get("/healthz")
    def healthz() -> dict[str, str]:
        return {"status": "ok"}

    @app.head("/upload")
    def upload_probe(authorization: str | None = Header(default=None)) -> None:
        authorize(authorization)

    @app.post("/upload")
    async def upload(clip: UploadFile = File(...), authorization: str | None = Header(default=None)) -> dict[str, str]:
        authorize(authorization)
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
        except HTTPException:
            target.unlink(missing_ok=True)
            raise
        except OSError as exc:
            target.unlink(missing_ok=True)
            raise HTTPException(status_code=500, detail="Could not store upload") from exc
        base = public_base_url.rstrip("/")
        if not base:
            base = ""  # FTHR will use the response URL only when configured publicly.
        return {"url": f"{base}/files/{identifier}", "id": identifier, "bytes": str(size)}

    @app.get("/files/{identifier}")
    def download(identifier: str) -> FileResponse:
        if not identifier.isalnum() and "-" not in identifier and "_" not in identifier:
            raise HTTPException(status_code=404, detail="Not found")
        target = storage / identifier
        if not target.is_file() or target.parent != storage:
            raise HTTPException(status_code=404, detail="Not found")
        return FileResponse(target, media_type="application/octet-stream", filename=identifier)

    return app


app = create_app(
    storage_dir=os.environ.get("STORAGE_DIR", "/data"),
    public_base_url=os.environ.get("PUBLIC_BASE_URL", ""),
    max_upload_bytes=int(os.environ.get("MAX_UPLOAD_BYTES", "524288000")),
)

if __name__ == "__main__":
    import uvicorn
    uvicorn.run("fthr_server:app", host=os.environ.get("BIND_HOST", "127.0.0.1"), port=int(os.environ.get("PORT", "8080")))
