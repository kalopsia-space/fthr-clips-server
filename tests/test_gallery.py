from pathlib import Path

from fastapi.testclient import TestClient

from fthr_server import create_app


def test_gallery_lists_uploaded_clips(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "secret", public_base_url="https://clips.example"))
    uploaded = client.post("/upload", headers={"Authorization": "Bearer secret"}, files={"clip": ("launch.mp4", b"video")})
    assert uploaded.status_code == 200

    response = client.get("/api/clips")
    assert response.status_code == 200
    assert response.json()["clips"][0]["name"] == "launch.mp4"
    assert response.json()["clips"][0]["url"].startswith("https://clips.example/files/")


def test_gallery_page_is_available(tmp_path: Path):
    client = TestClient(create_app(tmp_path, "secret"))
    response = client.get("/")
    assert response.status_code == 200
    assert "FTHR Clips" in response.text
    assert "api/clips" in response.text
