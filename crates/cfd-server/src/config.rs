//! Server configuration: where it listens, where it keeps data, and its limits.

use std::net::SocketAddr;
use std::path::PathBuf;

use cfd_engines::EngineOptions;
use serde::Serialize;

/// Hard limits on what a request may ask for (the browser sends whatever the user draws).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Grid cells per scene (4096² by default).
    pub max_cells: u64,
    /// Steps per job.
    pub max_steps: u64,
    /// Bytes of stored frames per job; past it the job keeps running without storing frames.
    pub max_stored_bytes: u64,
    /// Jobs waiting for a device; past it submissions are refused (`429`).
    pub max_queued_jobs: usize,
    /// Request body size (`413` past it).
    pub max_body_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_cells: 4096 * 4096,
            max_steps: 100_000_000,
            max_stored_bytes: 10 << 30,
            max_queued_jobs: 16,
            max_body_bytes: 64 << 20,
        }
    }
}

/// Defaults of the WebSocket stream.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamDefaults {
    /// Precision of streamed frames.
    pub stream_dtype: &'static str,
    /// Longest side of a streamed frame, in cells (frames are box-downsampled to fit).
    pub stream_max_size: u32,
}

impl Default for StreamDefaults {
    fn default() -> Self {
        Self {
            stream_dtype: "f16",
            stream_max_size: 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    /// Results go to `<data_dir>/results/<job id>/`.
    pub data_dir: PathBuf,
    /// sqlx URL of the database; default `sqlite://<data_dir>/live-fluids.db`.
    pub database_url: Option<String>,
    /// CPU jobs that may run at the same time (they share one rayon thread pool).
    pub cpu_jobs: usize,
    pub engine: EngineOptions,
    pub limits: Limits,
    pub stream: StreamDefaults,
    /// Origins allowed by CORS (the Vite development server by default).
    pub cors_origins: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 7878)),
            data_dir: PathBuf::from("data"),
            database_url: None,
            cpu_jobs: 1,
            engine: EngineOptions::default(),
            limits: Limits::default(),
            stream: StreamDefaults::default(),
            cors_origins: vec![
                "http://localhost:5173".into(),
                "http://127.0.0.1:5173".into(),
            ],
        }
    }
}

impl Config {
    pub fn results_dir(&self) -> PathBuf {
        self.data_dir.join("results")
    }

    pub fn database_url(&self) -> String {
        self.database_url.clone().unwrap_or_else(|| {
            format!(
                "sqlite://{}?mode=rwc",
                self.data_dir.join("live-fluids.db").display()
            )
        })
    }
}
