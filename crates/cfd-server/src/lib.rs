//! HTTP/WebSocket API, job manager and result storage of live-fluids.
//!
//! One process (ADR-006, ADR-010): axum on the tokio runtime handles I/O, and every running job
//! has its own OS threads for the solver and its output (`cfd-job`), so a solver never blocks the
//! async runtime and GPU contexts stay on their thread.
//!
//! - [`config`]: listen address, data directory, limits.
//! - [`api`]: the REST handlers.
//! - [`error`]: `ApiError` and its JSON form.
//! - [`jobs`]: job states, the job manager, per-device slots, the bridge to the job threads.
//! - [`stream`]: the latest frame per stream format, for WebSocket viewers.
//! - [`db`]: job and scene metadata in SQLite.

pub mod api;
pub mod config;
pub mod db;
pub mod error;
pub mod jobs;
pub mod stream;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderValue, Method};
use axum::routing::{get, post};
use axum::Router;
use cfd_engines::{EngineKind, Inventory};
use tokio::net::TcpListener;
use tower_http::compression::predicate::{NotForContentType, Predicate, SizeAbove};
use tower_http::compression::CompressionLayer;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub use config::{Config, Limits};

/// What every handler can reach. Cloning it clones an `Arc` (one shared instance, like a
/// singleton registered in ASP.NET's dependency injection); axum hands a clone to each handler
/// through the `State` extractor.
#[derive(Clone)]
pub struct AppState {
    pub inner: Arc<Inner>,
}

pub struct Inner {
    pub config: Config,
    pub inventory: Inventory,
    pub jobs: jobs::JobManager,
    /// Measured throughput per engine (MLUPS, smoothed over finished jobs), for time estimates.
    mlups: Mutex<HashMap<EngineKind, f64>>,
}

impl Inner {
    pub fn mlups(&self, engine: EngineKind) -> Option<f64> {
        // `std::sync::Mutex` is fine in async code as long as the guard is never held across an
        // `.await`: the critical section is a map lookup.
        self.mlups.lock().unwrap().get(&engine).copied()
    }

    pub fn record_mlups(&self, engine: EngineKind, mlups: f64) {
        if mlups.is_finite() && mlups > 0.0 {
            let mut m = self.mlups.lock().unwrap();
            let v = m.entry(engine).or_insert(mlups);
            *v = 0.5 * (*v + mlups);
        }
    }
}

impl AppState {
    /// Opens the database (marking jobs of an earlier run that never ended as interrupted) and
    /// the results directory.
    pub async fn new(config: Config, inventory: Inventory) -> Result<Self> {
        std::fs::create_dir_all(&config.data_dir)?;
        let db = db::Db::connect(&config.database_url()).await?;
        let mut mlups = HashMap::new();
        for (backend, m) in db.mlups_by_engine().await? {
            if let Some(k) = EngineKind::BY_PREFERENCE
                .into_iter()
                .find(|k| k.name() == backend)
            {
                mlups.insert(k, m);
            }
        }
        let jobs = jobs::JobManager::new(&config, &inventory, db).await?;
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                inventory,
                jobs,
                mlups: Mutex::new(mlups),
            }),
        })
    }
}

/// The routes and middleware (tower layers) of the API.
pub fn router(state: AppState) -> Router {
    let config = &state.inner.config;
    let origins: Vec<HeaderValue> = config
        .cors_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    let cors = CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers(Any);
    // JSON and CSV compress well; frames are already zstd-compressed.
    let compression = CompressionLayer::new().compress_when(
        SizeAbove::new(1024).and(NotForContentType::const_new("application/octet-stream")),
    );
    Router::new()
        .route("/api/capabilities", get(api::capabilities))
        .route("/api/scenes/validate", post(api::validate))
        .route("/api/scenes", get(api::list_scenes).post(api::save_scene))
        .route(
            "/api/scenes/{id}",
            get(api::get_scene).delete(api::delete_scene),
        )
        .route("/api/simulations", get(api::list_jobs).post(api::submit))
        .route(
            "/api/simulations/{id}",
            get(api::get_job).delete(api::delete_job),
        )
        .route("/api/simulations/{id}/pause", post(api::pause))
        .route("/api/simulations/{id}/resume", post(api::resume))
        .route("/api/simulations/{id}/frames/{n}", get(api::frame))
        .route("/api/simulations/{id}/files/{name}", get(api::file))
        .fallback(api::not_found)
        // Layers wrap the routes above, like ASP.NET middleware: the last one added runs first.
        .layer(DefaultBodyLimit::max(config.limits.max_body_bytes))
        .layer(compression)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Serves the API on `listener` until `shutdown` completes; then stops every job (recorded as
/// interrupted) before returning.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    let st = state.clone();
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            shutdown.await;
            st.inner.jobs.shutdown(Duration::from_secs(5)).await;
        })
        .await?;
    Ok(())
}
