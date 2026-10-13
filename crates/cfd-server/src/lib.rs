//! HTTP/WebSocket API, job manager and result storage of live-fluids.
//!
//! One process (ADR-006, ADR-010): axum on the tokio runtime handles I/O, and every running job
//! has its own OS threads for the solver and its output (`cfd-job`), so a solver never blocks the
//! async runtime and GPU contexts stay on their thread.
//!
//! - [`config`]: listen address, data directory, limits.
//! - [`api`]: the REST handlers.
//! - [`error`]: `ApiError` and its JSON form.

pub mod api;
pub mod config;
pub mod error;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
    pub async fn new(config: Config, inventory: Inventory) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                inventory,
                mlups: Mutex::default(),
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
        .fallback(api::not_found)
        // Layers wrap the routes above, like ASP.NET middleware: the last one added runs first.
        .layer(DefaultBodyLimit::max(config.limits.max_body_bytes))
        .layer(compression)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Serves the API on `listener` until `shutdown` completes.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}
