-- Saved scenes and the jobs run from them. Times are Unix milliseconds; frames and other results
-- live on disk (results/<job id>/), never in the database.

CREATE TABLE scenes (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    json        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE jobs (
    id            TEXT PRIMARY KEY,
    scene_id      TEXT REFERENCES scenes(id) ON DELETE SET NULL,
    scene_name    TEXT NOT NULL,
    scene_sha256  TEXT NOT NULL,
    -- queued | running | paused | completed | failed | cancelled
    status        TEXT NOT NULL,
    -- with `failed`: diverged | error | interrupted
    reason        TEXT,
    message       TEXT,
    backend       TEXT NOT NULL,
    device        TEXT,
    options_json  TEXT NOT NULL,
    steps_done    INTEGER NOT NULL DEFAULT 0,
    steps_total   INTEGER NOT NULL,
    frames        INTEGER NOT NULL DEFAULT 0,
    mlups         REAL,
    created_at    INTEGER NOT NULL,
    started_at    INTEGER,
    finished_at   INTEGER,
    result_dir    TEXT NOT NULL
);

CREATE INDEX jobs_by_status ON jobs (status);
CREATE INDEX jobs_by_creation ON jobs (created_at);
