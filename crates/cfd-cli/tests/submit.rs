//! `cfd-cli submit` against a `cfd-server` started in this process (CPU engine).

mod common;

use std::path::Path;
use std::process::{Command, Output};

use cfd_core::examples;
use cfd_core::Scene;
use cfd_engines::{EngineKind, EngineOptions, Inventory};
use cfd_server::{AppState, Config};
use common::{small_channel, temp_dir};
use tokio::sync::oneshot;

/// Starts a CPU-only server on an ephemeral port; returns its URL and a stop switch.
async fn server(data: &Path) -> (String, oneshot::Sender<()>) {
    let config = Config {
        data_dir: data.to_path_buf(),
        database_url: Some("sqlite::memory:".into()),
        ..Default::default()
    };
    let inventory = Inventory::probe_only(&[EngineKind::Cpu], &EngineOptions::default());
    let state = AppState::new(config, inventory).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = oneshot::channel::<()>();
    tokio::spawn(cfd_server::serve(listener, state, async {
        let _ = stopped.await;
    }));
    (url, stop)
}

/// Runs `cfd-cli submit` on a scene (on a blocking thread, so the server keeps running).
async fn submit(dir: &Path, scene: &Scene, url: &str, extra: &[&str]) -> Output {
    let path = dir.join("scene.json");
    std::fs::write(&path, scene.to_json_pretty()).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cfd-cli"));
    cmd.arg("submit")
        .arg(&path)
        .args(["--server", url])
        .args(extra);
    tokio::task::spawn_blocking(move || cmd.output().unwrap())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn submit_follows_a_job_to_completion() {
    let dir = temp_dir("submit-ok");
    let (url, stop) = server(&dir.join("data")).await;
    let out = submit(
        &dir,
        &small_channel(),
        &url,
        &["--end-time", "0.2", "--output-interval", "0.05"],
    )
    .await;
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("on cpu"), "{err}");
    assert!(
        err.contains("done: completed — 200 steps, 5 frames"),
        "{err}"
    );

    // A scene the server refuses: exit code 1 with the issue codes.
    let mut bad = small_channel();
    bad.grid.width = 65;
    let out = submit(&dir, &bad, &url, &[]).await;
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains("scene.invalid") && err.contains("layer.cellType"),
        "{err}"
    );
    let _ = stop.send(());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn submit_cancels_after_a_while() {
    let dir = temp_dir("submit-cancel");
    let (url, stop) = server(&dir.join("data")).await;
    let out = submit(
        &dir,
        &examples::channel(),
        &url,
        &[
            "--end-time",
            "100",
            "--output-interval",
            "0.05",
            "--cancel-after",
            "1",
        ],
    )
    .await;
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert!(err.contains("first frame"), "{err}");
    assert!(err.contains("cancel took effect after 0."), "{err}");
    assert!(err.contains("done: cancelled"), "{err}");
    let _ = stop.send(());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn submit_says_when_there_is_no_server() {
    let dir = temp_dir("submit-none");
    // A port nothing listens on: bind one, then free it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let out = submit(
        &dir,
        &small_channel(),
        &format!("http://127.0.0.1:{port}"),
        &[],
    )
    .await;
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1));
    assert!(err.contains("is cfd-server running?"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
