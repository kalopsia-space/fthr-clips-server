from pathlib import Path

from fastapi.testclient import TestClient

from fthr_server import create_app


def test_upload_requires_bearer_token(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "secret"))
    response = client.post("/upload", files={"clip": ("clip.mp4", b"video")})
    assert response.status_code == 401


def test_upload_returns_downloadable_file(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "secret", public_base_url="https://clips.example"))
    response = client.post("/upload", headers={"Authorization": "Bearer secret"}, files={"clip": ("clip.mp4", b"video")})
    assert response.status_code == 200
    body = response.json()
    assert body["url"].startswith("https://clips.example/files/")
    downloaded = client.get(f"/files/{body['id']}")
    assert downloaded.status_code == 200
    assert downloaded.content == b"video"


def test_upload_rejects_oversized_file(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "secret", max_upload_bytes=3))
    response = client.post("/upload", headers={"Authorization": "Bearer secret"}, files={"clip": ("clip.mp4", b"four")})
    assert response.status_code == 413
    assert not list(tmp_path.iterdir())


def test_head_connection_check_requires_auth(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "secret"))
    assert client.head("/upload").status_code == 401
    assert client.head("/upload", headers={"Authorization": "Bearer secret"}).status_code == 200
