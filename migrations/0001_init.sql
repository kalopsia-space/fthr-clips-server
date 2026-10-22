CREATE TABLE media (
    id TEXT PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('video', 'image')),
    path TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('scan', 'upload')),
    folder TEXT NOT NULL DEFAULT '',
    extension TEXT NOT NULL DEFAULT '',
    size INTEGER NOT NULL DEFAULT 0,
    available INTEGER NOT NULL DEFAULT 1,
    title TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    public INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    recorded_at TEXT,
    duration_ms INTEGER,
    width INTEGER,
    height INTEGER,
    codec_video TEXT,
    codec_audio TEXT,
    container TEXT,
    fps REAL,
    bitrate INTEGER,
    probe_json TEXT,
    has_poster INTEGER NOT NULL DEFAULT 0,
    has_preview INTEGER NOT NULL DEFAULT 0,
    has_thumb INTEGER NOT NULL DEFAULT 0,
    processing_state TEXT NOT NULL DEFAULT 'pending'
        CHECK (processing_state IN ('pending', 'ready', 'failed')),
    processing_error TEXT
);
CREATE INDEX idx_media_slug ON media(slug);
CREATE INDEX idx_media_path ON media(path);
CREATE INDEX idx_media_public_created ON media(public, created_at);

CREATE TABLE tag (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    colour TEXT NOT NULL DEFAULT '#20e3a0'
);

CREATE TABLE media_tag (
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tag(id) ON DELETE CASCADE,
    PRIMARY KEY (media_id, tag_id)
);

CREATE TABLE view (
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    ip_hash TEXT NOT NULL,
    first_seen TEXT NOT NULL,
    PRIMARY KEY (media_id, ip_hash)
);

CREATE TABLE session (
    id TEXT PRIMARY KEY,
    csrf TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE setting (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE upload_token (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    prefix TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    revoked_at TEXT
);

CREATE TABLE job (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'running', 'done', 'failed', 'cancelled')),
    attempts INTEGER NOT NULL DEFAULT 0,
    progress REAL NOT NULL DEFAULT 0,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_job_state_kind ON job(state, kind);
CREATE INDEX idx_job_media ON job(media_id);
CREATE UNIQUE INDEX idx_job_active ON job(media_id, kind) WHERE state IN ('queued', 'running');

CREATE TABLE rendition (
    media_id TEXT NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    height INTEGER NOT NULL,
    path TEXT NOT NULL DEFAULT '',
    size INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'queued',
    progress REAL NOT NULL DEFAULT 0,
    error TEXT,
    PRIMARY KEY (media_id, height)
);

CREATE TABLE legacy_id (
    old_id TEXT PRIMARY KEY,
    media_id TEXT NOT NULL
);

CREATE TABLE tombstone (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL
);

CREATE TABLE webhook_sent (
    media_id TEXT PRIMARY KEY
);
