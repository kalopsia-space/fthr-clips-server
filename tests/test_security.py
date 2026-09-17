from pathlib import Path

from fastapi.testclient import TestClient

from fthr_server import create_app


def test_upload_requires_api_key_and_rate_limits(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "upload-key", admin_password="admin-pass", upload_limit=1))
    files = {"clip": ("clip.mp4", b"video")}
    assert client.post("/upload", files=files).status_code == 401
    assert client.post("/upload", headers={"Authorization": "Bearer upload-key"}, files=files).status_code == 200
    assert client.post("/upload", headers={"Authorization": "Bearer upload-key"}, files=files).status_code == 429


def test_admin_login_and_unlisted_clip_management(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "upload-key", admin_password="admin-pass"))
    uploaded = client.post("/upload", headers={"Authorization": "Bearer upload-key"}, files={"clip": ("clip.mp4", b"video")})
    identifier = uploaded.json()["id"]
    assert client.get("/api/clips").json()["clips"]
    assert client.post("/admin/login", json={"password": "wrong"}).status_code == 401
    login = client.post("/admin/login", json={"password": "admin-pass"})
    assert login.status_code == 200
    csrf = login.json()["csrf"]
    assert client.get("/admin/api/clips").json()["clips"]
    assert client.patch(f"/admin/api/clips/{identifier}", headers={"X-CSRF-Token": csrf}, json={"listed": False}).status_code == 200
    assert client.get("/api/clips").json()["clips"] == []
    assert client.delete(f"/admin/api/clips/{identifier}", headers={"X-CSRF-Token": csrf}).status_code == 200
    assert client.get(f"/files/{identifier}").status_code == 404


def test_admin_dashboard_is_gated(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "upload-key", admin_password="admin-pass"))
    assert client.get("/admin").status_code == 200
    client.post("/admin/login", json={"password": "admin-pass"})
    response = client.get("/admin/dashboard")
    assert response.status_code == 200
    assert "Manage your clips" in response.text
