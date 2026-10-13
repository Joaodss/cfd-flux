//! A server started in-process on an ephemeral port, with a temporary data directory.
#![allow(dead_code)]

use std::path::PathBuf;

use cfd_core::examples::{self, Canvas};
use cfd_core::scene::CellType;
use cfd_core::Scene;
use cfd_engines::{EngineKind, EngineOptions, Inventory};
use cfd_server::{AppState, Config};
use tokio::sync::oneshot;

pub struct TestServer {
    pub base: String,
    pub data: PathBuf,
    pub state: AppState,
    stop: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl TestServer {
    /// A server with only the CPU engine.
    pub async fn start(name: &str) -> Self {
        Self::with(name, &[EngineKind::Cpu], |_| {}).await
    }

    /// A server with the given engines, after `tweak` has changed its configuration.
    pub async fn with(name: &str, engines: &[EngineKind], tweak: impl FnOnce(&mut Config)) -> Self {
        let data =
            std::env::temp_dir().join(format!("cfd-server-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let mut config = Config {
            data_dir: data.clone(),
            database_url: Some("sqlite::memory:".into()),
            engine: EngineOptions {
                allow_software: true,
                ..Default::default()
            },
            ..Default::default()
        };
        tweak(&mut config);
        let opts = config.engine;
        let engines = engines.to_vec();
        let inventory = tokio::task::spawn_blocking(move || Inventory::probe_only(&engines, &opts))
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = AppState::new(config, inventory).await.unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let task = tokio::spawn(cfd_server::serve(listener, state.clone(), async {
            let _ = stopped.await;
        }))
        .map_ok();
        Self {
            base,
            data,
            state,
            stop: Some(stop),
            task: Some(task),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn ws_url(&self, path: &str) -> String {
        format!("{}{path}", self.base.replace("http://", "ws://"))
    }

    pub async fn stop(mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

/// Turns the `serve` task's `Result` into `()` (a failing server makes the requests fail).
trait MapOk {
    fn map_ok(self) -> tokio::task::JoinHandle<()>;
}

impl MapOk for tokio::task::JoinHandle<anyhow::Result<()>> {
    fn map_ok(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Ok(Err(e)) = self.await {
                panic!("server failed: {e:#}");
            }
        })
    }
}

/// A 64 × 18 channel (walls, uniform inlet, pressure outlet) in the units of the `channel`
/// example (dx = 1 mm, dt = 1 ms).
pub fn small_channel() -> Scene {
    let (w, h) = (64, 18);
    let mut c = Canvas::new(w, h, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, 1);
    c.rect(0, h - 1, w, h, CellType::Solid, 1);
    c.rect(0, 1, 1, h - 1, CellType::Inlet, 2);
    c.rect(w - 1, 1, w, h - 1, CellType::Outlet, 3);
    let mut scene = examples::channel();
    scene.grid.width = w;
    scene.grid.height = h;
    scene.layers = c.layers();
    scene.probes[0].position = [32, 9];
    scene
}

pub fn client() -> reqwest::Client {
    reqwest::Client::new()
}
