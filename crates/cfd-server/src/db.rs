//! Job and scene metadata in SQLite, through `sqlx`.
//!
//! Queries are plain strings checked at runtime (`sqlx::query`, not the `query!` macros), so the
//! build needs no database. Every call is `async`: sqlx runs SQLite on its own worker thread and
//! the handler's task is suspended meanwhile, never blocking the runtime (like `await`ing an
//! EF Core call in C#). The schema is in `migrations/`, embedded in the binary by
//! `sqlx::migrate!` and applied on start-up.

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::FromRow;

/// Unix time in milliseconds.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// One row of `jobs`.
#[derive(Debug, Clone, FromRow)]
pub struct JobRow {
    pub id: String,
    pub scene_id: Option<String>,
    pub scene_name: String,
    pub scene_sha256: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub backend: String,
    pub device: Option<String>,
    pub options_json: String,
    pub steps_done: i64,
    pub steps_total: i64,
    pub frames: i64,
    pub mlups: Option<f64>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub result_dir: String,
}

/// How a job ended, for [`Db::finish_job`].
pub struct JobEnd<'a> {
    pub status: &'a str,
    pub reason: Option<&'a str>,
    pub message: Option<&'a str>,
    pub steps_done: u64,
    pub frames: u32,
    pub mlups: Option<f64>,
}

/// One row of `scenes` without its JSON.
#[derive(Debug, Clone, FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneSummary {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A pool of SQLite connections. Cloning it is cheap (an `Arc` inside).
#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    /// Opens (creating it if needed) and migrates the database at `url`, e.g.
    /// `sqlite://data/live-fluids.db?mode=rwc` or `sqlite::memory:`.
    pub async fn connect(url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(url)
            .with_context(|| format!("invalid database URL {url:?}"))?
            .foreign_keys(true);
        // An in-memory database exists once per connection: keep exactly one.
        let max = if url.contains(":memory:") { 1 } else { 4 };
        let pool = SqlitePoolOptions::new()
            .max_connections(max)
            .connect_with(options)
            .await
            .with_context(|| format!("cannot open the database {url:?}"))?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("cannot migrate the database")?;
        Ok(Self { pool })
    }

    /// Jobs still active when the server stopped cannot be resumed (no checkpoints yet): they
    /// become `failed/interrupted`. Returns how many there were.
    pub async fn mark_interrupted(&self) -> Result<u64> {
        let r = sqlx::query(
            "UPDATE jobs SET status = 'failed', reason = 'interrupted',
                 message = 'the server stopped while the job was active', finished_at = ?
             WHERE status IN ('queued', 'running', 'paused')",
        )
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    pub async fn insert_job(&self, j: &JobRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO jobs (id, scene_id, scene_name, scene_sha256, status, reason, message,
                 backend, device, options_json, steps_done, steps_total, frames, mlups,
                 created_at, started_at, finished_at, result_dir)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&j.id)
        .bind(&j.scene_id)
        .bind(&j.scene_name)
        .bind(&j.scene_sha256)
        .bind(&j.status)
        .bind(&j.reason)
        .bind(&j.message)
        .bind(&j.backend)
        .bind(&j.device)
        .bind(&j.options_json)
        .bind(j.steps_done)
        .bind(j.steps_total)
        .bind(j.frames)
        .bind(j.mlups)
        .bind(j.created_at)
        .bind(j.started_at)
        .bind(j.finished_at)
        .bind(&j.result_dir)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn start_job(&self, id: &str, at: i64) -> Result<()> {
        sqlx::query("UPDATE jobs SET status = 'running', started_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_status(&self, id: &str, status: &str) -> Result<()> {
        sqlx::query("UPDATE jobs SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn finish_job(&self, id: &str, end: &JobEnd<'_>) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET status = ?, reason = ?, message = ?, steps_done = ?, frames = ?,
                 mlups = ?, finished_at = ?
             WHERE id = ?",
        )
        .bind(end.status)
        .bind(end.reason)
        .bind(end.message)
        .bind(end.steps_done as i64)
        .bind(end.frames as i64)
        .bind(end.mlups)
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn job(&self, id: &str) -> Result<Option<JobRow>> {
        Ok(
            sqlx::query_as::<_, JobRow>("SELECT * FROM jobs WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Jobs, newest first, optionally only those with `status`.
    pub async fn jobs(&self, status: Option<&str>, limit: u32) -> Result<Vec<JobRow>> {
        Ok(sqlx::query_as::<_, JobRow>(
            "SELECT * FROM jobs WHERE (?1 IS NULL OR status = ?1)
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn delete_job(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM jobs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Mean throughput of finished jobs per engine (MLUPS), for time estimates.
    pub async fn mlups_by_engine(&self) -> Result<Vec<(String, f64)>> {
        Ok(sqlx::query_as::<_, (String, f64)>(
            "SELECT backend, AVG(mlups) FROM jobs
             WHERE status = 'completed' AND mlups IS NOT NULL GROUP BY backend",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn insert_scene(&self, id: &str, name: &str, json: &str) -> Result<()> {
        let now = now_ms();
        sqlx::query(
            "INSERT INTO scenes (id, name, json, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn scenes(&self) -> Result<Vec<SceneSummary>> {
        Ok(sqlx::query_as::<_, SceneSummary>(
            "SELECT id, name, created_at, updated_at FROM scenes ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn scene_json(&self, id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar::<_, String>("SELECT json FROM scenes WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Returns whether the scene existed.
    pub async fn delete_scene(&self, id: &str) -> Result<bool> {
        let r = sqlx::query("DELETE FROM scenes WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}
