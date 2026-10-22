-- "Open with results" (the gallery): the latest completed job of the same scene *and* the same run
-- options is reused instead of running again. `scene_hash` is the SHA-256 of the validated scene's
-- canonical JSON plus the options that change the result (see `api::run_hash`); NULL for jobs
-- recorded before this migration, which are never reused.
ALTER TABLE jobs ADD COLUMN scene_hash TEXT;

CREATE INDEX jobs_by_scene_hash ON jobs (scene_hash, status, created_at);
