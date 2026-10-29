//! Jobs over REST on the CPU engine: lifecycle, cancel, pause/resume, queueing, results, saved
//! scenes, shutdown.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cfd_core::examples;
use cfd_core::Scene;
use cfd_engines::{Engine, EngineKind, EngineOptions, SolverChoice};
use cfd_io::frame::{Dtype, Frame};
use common::{client, small_channel, TestServer};
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn submit(s: &TestServer, scene: &Scene, options: Value) -> Value {
    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "scene": scene, "options": options }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        StatusCode::CREATED,
        "{}",
        r.text().await.unwrap()
    );
    let location = r.headers()["location"].to_str().unwrap().to_string();
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        location,
        format!("/api/simulations/{}", v["id"].as_str().unwrap())
    );
    v
}

async fn get(s: &TestServer, id: &str) -> Value {
    client()
        .get(s.url(&format!("/api/simulations/{id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Polls the job until `pred` holds (at most 60 s).
async fn wait_until(s: &TestServer, id: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let v = get(s, id).await;
        if pred(&v) {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out; last state {v}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn status_is(status: &'static str) -> impl Fn(&Value) -> bool {
    move |v| v["status"] == status
}

/// The channel example (400 × 100) for 100 s of flow: minutes of CPU work.
fn long_options() -> Value {
    json!({ "endTime": 100.0, "outputInterval": 0.5 })
}

#[tokio::test]
async fn a_job_runs_to_completion_and_its_results_are_served() {
    let s = TestServer::start("complete").await;
    let job = submit(
        &s,
        &small_channel(),
        json!({ "endTime": 0.2, "outputInterval": 0.05 }),
    )
    .await;
    let id = job["id"].as_str().unwrap().to_string();
    assert_eq!(job["backend"], "cpu");
    assert_eq!(job["stepsTotal"], 200);
    assert_eq!(job["estimate"]["cells"], 64 * 18);

    let done = wait_until(&s, &id, status_is("completed")).await;
    assert_eq!(done["stepsDone"], 200);
    assert_eq!(done["frames"], 5);
    assert!(done["mlups"].as_f64().unwrap() > 0.0);
    assert!(done["startedAt"].as_i64().unwrap() >= done["createdAt"].as_i64().unwrap());

    let meta: Value = client()
        .get(s.url(&format!("/api/simulations/{id}/files/meta.json")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(meta["run"]["status"], "completed");
    assert_eq!(
        meta["generator"],
        format!("cfd-server {}", env!("CARGO_PKG_VERSION"))
    );
    let csv = client()
        .get(s.url(&format!("/api/simulations/{id}/files/probes.csv")))
        .send()
        .await
        .unwrap();
    assert_eq!(csv.headers()["content-type"], "text/csv; charset=utf-8");
    assert_eq!(csv.text().await.unwrap().lines().count(), 42);
    let bad = client()
        .get(s.url(&format!("/api/simulations/{id}/files/..%2Fsecret")))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::NOT_FOUND);

    // Stored frames, as stored (f32) or converted to f16.
    for (dtype, expected) in [("f32", Dtype::F32), ("f16", Dtype::F16)] {
        let r = client()
            .get(s.url(&format!("/api/simulations/{id}/frames/4?dtype={dtype}")))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        let f = Frame::decode(&r.bytes().await.unwrap()).unwrap();
        assert_eq!((f.frame_index, f.step, f.width), (4, 200, 64));
        assert_eq!(f.job_id, id);
        assert!(f.fields.iter().all(|f| f.desc.dtype == expected));
    }
    // Without `maxSize`, f16 frames (the viewer's full-resolution "sharpen", with or without
    // `fields`) hold every cell: each value is the f16 rounding of the stored f32 value.
    let frame = |query: &'static str| {
        let url = s.url(&format!("/api/simulations/{id}/frames/4{query}"));
        async move {
            let r = client().get(url).send().await.unwrap();
            assert_eq!(r.status(), StatusCode::OK);
            Frame::decode(&r.bytes().await.unwrap()).unwrap()
        }
    };
    let stored = frame("").await;
    for query in ["?dtype=f16", "?dtype=f16&fields=velocity,pressure"] {
        let half = frame(query).await;
        assert_eq!((half.width, half.height), (stored.width, stored.height));
        assert!(!half.fields.is_empty());
        for f in &half.fields {
            let s = stored
                .fields
                .iter()
                .find(|s| s.desc.name == f.desc.name)
                .unwrap();
            assert_eq!(f.desc.dtype, Dtype::F16);
            assert_eq!(f.values.len(), s.values.len());
            for (h, v) in f.values.iter().zip(&s.values) {
                assert_eq!(h.to_bits(), half::f16::from_f32(*v).to_f32().to_bits());
            }
        }
    }
    let missing = client()
        .get(s.url(&format!("/api/simulations/{id}/frames/5")))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    // Listed, and the measured throughput now feeds the time estimate.
    let list: Value = client()
        .get(s.url("/api/simulations?status=completed"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["id"], id.as_str());
    let v: Value = client()
        .post(s.url("/api/scenes/validate"))
        .json(&small_channel())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(v["estimate"]["seconds"].as_f64().unwrap() > 0.0, "{v}");

    // A finished job cannot be cancelled, paused or resumed.
    for (method, path) in [
        (reqwest::Method::DELETE, ""),
        (reqwest::Method::POST, "/pause"),
        (reqwest::Method::POST, "/resume"),
    ] {
        let r = client()
            .request(method, s.url(&format!("/api/simulations/{id}{path}")))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CONFLICT, "{path}");
    }
    s.stop().await;
}

#[tokio::test]
async fn cancelling_a_running_job_takes_less_than_a_second() {
    let s = TestServer::start("cancel").await;
    let job = submit(&s, &examples::channel(), long_options()).await;
    let id = job["id"].as_str().unwrap().to_string();
    wait_until(&s, &id, |v| v["stepsDone"].as_u64().unwrap_or(0) > 50).await;
    let t = Instant::now();
    let r = client()
        .delete(s.url(&format!("/api/simulations/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let v = wait_until(&s, &id, status_is("cancelled")).await;
    let latency = t.elapsed();
    eprintln!("cancelled in {latency:?} at step {}", v["stepsDone"]);
    assert!(latency < Duration::from_secs(1), "{latency:?}");
    let meta: Value = serde_json::from_str(
        &std::fs::read_to_string(s.data.join(format!("results/{id}/meta.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["run"]["status"], "cancelled");
    s.stop().await;
}

#[tokio::test]
async fn pause_stops_progress_until_resume() {
    let s = TestServer::start("pause").await;
    let job = submit(&s, &examples::channel(), long_options()).await;
    let id = job["id"].as_str().unwrap().to_string();
    wait_until(&s, &id, |v| v["stepsDone"].as_u64().unwrap_or(0) > 0).await;
    let r = client()
        .post(s.url(&format!("/api/simulations/{id}/pause")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let paused = wait_until(&s, &id, status_is("paused")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(get(&s, &id).await["stepsDone"], paused["stepsDone"]);

    client()
        .post(s.url(&format!("/api/simulations/{id}/resume")))
        .send()
        .await
        .unwrap();
    let steps = paused["stepsDone"].as_u64().unwrap();
    wait_until(&s, &id, |v| {
        v["status"] == "running" && v["stepsDone"].as_u64().unwrap() > steps
    })
    .await;
    client()
        .delete(s.url(&format!("/api/simulations/{id}")))
        .send()
        .await
        .unwrap();
    wait_until(&s, &id, status_is("cancelled")).await;
    s.stop().await;
}

/// One CPU slot: a second job waits for the first; cancelling it while queued is immediate.
#[tokio::test]
async fn jobs_queue_for_their_device() {
    let s = TestServer::start("queue").await;
    let first = submit(&s, &examples::channel(), long_options()).await;
    let second = submit(&s, &examples::channel(), long_options()).await;
    let (a, b) = (
        first["id"].as_str().unwrap(),
        second["id"].as_str().unwrap(),
    );
    wait_until(&s, a, status_is("running")).await;
    assert_eq!(get(&s, b).await["status"], "queued");
    // Pausing a job that has not started is refused.
    let r = client()
        .post(s.url(&format!("/api/simulations/{b}/pause")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);

    let r = client()
        .delete(s.url(&format!("/api/simulations/{b}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["status"], "cancelled");
    assert!(v["startedAt"].is_null());
    assert_eq!(get(&s, a).await["status"], "running");

    // A third job starts once the first is cancelled.
    let third = submit(&s, &small_channel(), json!({ "endTime": 0.05 })).await;
    let c = third["id"].as_str().unwrap();
    assert_eq!(get(&s, c).await["status"], "queued");
    client()
        .delete(s.url(&format!("/api/simulations/{a}")))
        .send()
        .await
        .unwrap();
    wait_until(&s, c, status_is("completed")).await;
    s.stop().await;
}

#[tokio::test]
async fn submissions_that_cannot_run_are_refused() {
    let s = TestServer::with("refused", &[EngineKind::Cpu], |c| {
        c.limits.max_queued_jobs = 1;
    })
    .await;
    let mut bad = small_channel();
    bad.fluids[0].kinematic_viscosity = -1.0;
    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "scene": bad }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "scene.invalid");
    assert!(!v["error"]["issues"].as_array().unwrap().is_empty());

    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "scene": small_channel(), "options": { "backend": "cuda" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"]["code"], "engine.unavailable");

    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "scene": small_channel(), "options": { "speed": 2 } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "unknown option");

    // The queue holds one job: a running one, a queued one, then 429.
    let a = submit(&s, &examples::channel(), long_options()).await;
    wait_until(&s, a["id"].as_str().unwrap(), status_is("running")).await;
    submit(&s, &examples::channel(), long_options()).await;
    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "scene": small_channel() }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
    s.stop().await;
}

#[tokio::test]
async fn purge_deletes_results_and_record() {
    let s = TestServer::start("purge").await;
    let job = submit(&s, &small_channel(), json!({ "endTime": 0.05 })).await;
    let id = job["id"].as_str().unwrap().to_string();
    wait_until(&s, &id, status_is("completed")).await;
    let dir = s.data.join(format!("results/{id}"));
    assert!(dir.join("meta.json").exists());
    let r = client()
        .delete(s.url(&format!("/api/simulations/{id}?purge=true")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert!(!dir.exists());
    let r = client()
        .get(s.url(&format!("/api/simulations/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    // A running job is cancelled, then purged once it has stopped.
    let job = submit(&s, &examples::channel(), long_options()).await;
    let id = job["id"].as_str().unwrap().to_string();
    wait_until(&s, &id, status_is("running")).await;
    let r = client()
        .delete(s.url(&format!("/api/simulations/{id}?purge=true")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::ACCEPTED);
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.data.join(format!("results/{id}")).exists() {
        assert!(Instant::now() < deadline, "not purged");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    s.stop().await;
}

#[tokio::test]
async fn saved_scenes_can_be_run_by_id() {
    let s = TestServer::start("scenes").await;
    let r = client()
        .post(s.url("/api/scenes"))
        .json(&small_channel())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let saved: Value = r.json().await.unwrap();
    let scene_id = saved["id"].as_str().unwrap().to_string();
    assert_eq!(saved["name"], small_channel().name);
    let list: Value = client()
        .get(s.url("/api/scenes"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list[0]["id"], scene_id.as_str());
    let back: Scene = client()
        .get(s.url(&format!("/api/scenes/{scene_id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(back, small_channel());

    // PUT replaces it (name and content); an unknown id is a 404.
    let mut renamed = small_channel();
    renamed.name = "Renamed channel".into();
    let r = client()
        .put(s.url(&format!("/api/scenes/{scene_id}")))
        .json(&renamed)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        (v["id"].as_str(), v["name"].as_str()),
        (Some(scene_id.as_str()), Some("Renamed channel"))
    );
    let back: Scene = client()
        .get(s.url(&format!("/api/scenes/{scene_id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(back, renamed);
    let r = client()
        .put(s.url("/api/scenes/no-such-scene"))
        .json(&renamed)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);

    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "sceneId": scene_id, "options": { "endTime": 0.05 } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
    let job: Value = r.json().await.unwrap();
    assert_eq!(job["sceneId"], scene_id.as_str());
    wait_until(&s, job["id"].as_str().unwrap(), status_is("completed")).await;

    let r = client()
        .delete(s.url(&format!("/api/scenes/{scene_id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    // The job keeps its results; its scene reference is cleared.
    let v = get(&s, job["id"].as_str().unwrap()).await;
    assert_eq!(v["status"], "completed");
    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "sceneId": scene_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    s.stop().await;
}

/// Stopping the server stops its jobs and records them as interrupted; a new server on the same
/// database reports them so.
#[tokio::test]
async fn jobs_active_at_shutdown_are_interrupted() {
    let db_dir = std::env::temp_dir().join(format!("cfd-server-test-{}-db", std::process::id()));
    let _ = std::fs::remove_dir_all(&db_dir);
    std::fs::create_dir_all(&db_dir).unwrap();
    let url = format!("sqlite://{}?mode=rwc", db_dir.join("jobs.db").display());
    let u = url.clone();
    let s = TestServer::with("shutdown", &[EngineKind::Cpu], move |c| {
        c.database_url = Some(u);
    })
    .await;
    let running = submit(&s, &examples::channel(), long_options()).await;
    let queued = submit(&s, &examples::channel(), long_options()).await;
    let (a, b) = (
        running["id"].as_str().unwrap().to_string(),
        queued["id"].as_str().unwrap().to_string(),
    );
    wait_until(&s, &a, status_is("running")).await;
    let state = s.state.clone();
    let t = Instant::now();
    s.stop().await;
    assert!(t.elapsed() < Duration::from_secs(5));
    for id in [&a, &b] {
        let v = state.inner.jobs.view(id).await.unwrap();
        assert_eq!(
            (v.status.as_str(), v.reason.map(|r| r.as_str())),
            ("failed", Some("interrupted"))
        );
    }

    let s2 = TestServer::with("shutdown-2", &[EngineKind::Cpu], move |c| {
        c.database_url = Some(url);
    })
    .await;
    for id in [&a, &b] {
        let v = get(&s2, id).await;
        assert_eq!(v["status"], "failed");
        assert_eq!(v["reason"], "interrupted");
    }
    s2.stop().await;
    let _ = std::fs::remove_dir_all(&db_dir);
}

/// A server job writes what `cfd-cli run` writes (both are `cfd_job::run`): the same CSV series
/// byte for byte, and frames with the same fields (only the job id in the header differs).
#[tokio::test]
async fn server_results_match_a_direct_run() {
    let s = TestServer::start("same").await;
    let options = json!({ "endTime": 0.2, "outputInterval": 0.05 });
    let job = submit(&s, &small_channel(), options).await;
    let id = job["id"].as_str().unwrap().to_string();
    wait_until(&s, &id, status_is("completed")).await;
    let server_dir = s.data.join(format!("results/{id}"));

    let direct = s.data.join("direct");
    let opts = cfd_job::RunOptions {
        end_time: Some(0.2),
        output_interval: Some(0.05),
        ..Default::default()
    };
    let d = direct.clone();
    tokio::task::spawn_blocking(move || {
        let prepared = cfd_job::prepare(small_channel().to_json_pretty(), opts).unwrap();
        let dir = cfd_job::ResultDir::create(&d, false).unwrap();
        let choice = SolverChoice::new(Engine::Cpu, &EngineOptions::default());
        let (_, rx) = std::sync::mpsc::channel();
        let job = cfd_job::JobContext::new("direct", "test");
        cfd_job::run(
            prepared,
            &choice,
            &dir,
            &job,
            Arc::new(cfd_job::NoObserver),
            &rx,
        )
        .unwrap();
    })
    .await
    .unwrap();

    let read = |dir: &Path, f: &str| std::fs::read(dir.join(f)).unwrap();
    for f in ["probes.csv", "forces.csv", "diagnostics.csv"] {
        assert!(read(&server_dir, f) == read(&direct, f), "{f} differs");
    }
    for i in 0..5 {
        let name = format!("frames/{i:06}.bin");
        let a = Frame::decode(&read(&server_dir, &name)).unwrap();
        let b = Frame::decode(&read(&direct, &name)).unwrap();
        assert_eq!((a.step, a.width, a.height), (b.step, b.width, b.height));
        assert_eq!(a.fields, b.fields, "{name}");
    }
    s.stop().await;
}

/// `POST /api/simulations?reuse=completed`: the gallery's "Open with results".
#[tokio::test]
async fn a_completed_job_of_the_same_scene_and_options_is_reused_on_request() {
    let s = TestServer::start("reuse").await;
    let scene = small_channel();
    let options = json!({ "endTime": 0.1, "outputInterval": 0.05 });
    let post = |scene: Value, options: Value, query: &'static str| {
        let url = s.url(&format!("/api/simulations{query}"));
        async move {
            client()
                .post(url)
                .json(&json!({ "scene": scene, "options": options }))
                .send()
                .await
                .unwrap()
        }
    };
    let scene_json = serde_json::to_value(&scene).unwrap();

    // Nothing has run yet: a new job (201), also with `reuse`.
    let r = post(scene_json.clone(), options.clone(), "?reuse=completed").await;
    assert_eq!(r.status(), StatusCode::CREATED);
    let first: Value = r.json().await.unwrap();
    let id = first["id"].as_str().unwrap().to_string();

    // Not completed yet (it is queued or running): still a new job, never a half-done one. The
    // short run may already be over, so only check when the first job was seen active.
    let active = get(&s, &id).await["status"] != "completed";
    if active {
        let r = post(scene_json.clone(), options.clone(), "?reuse=completed").await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let other: Value = r.json().await.unwrap();
        assert_ne!(other["id"], first["id"]);
        wait_until(&s, other["id"].as_str().unwrap(), status_is("completed")).await;
    }
    wait_until(&s, &id, status_is("completed")).await;

    // Completed: the same scene and options answer with the latest completed job (200).
    let r = post(scene_json.clone(), options.clone(), "?reuse=completed").await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.headers()["location"]
        .to_str()
        .unwrap()
        .starts_with("/api/simulations/"));
    let again: Value = r.json().await.unwrap();
    assert_eq!(again["status"], "completed");
    let listed = client()
        .get(s.url("/api/simulations"))
        .send()
        .await
        .unwrap()
        .json::<Vec<Value>>()
        .await
        .unwrap();
    assert!(listed.iter().any(|j| j["id"] == again["id"]));

    // The engine request does not matter, nor how the scene was formatted (the hash is of the
    // parsed scene)...
    let pretty = serde_json::from_str::<Value>(&scene.to_json_pretty()).unwrap();
    let mut opts = options.clone();
    opts["backend"] = json!("cpu");
    let r = post(pretty, opts, "?reuse=completed").await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.json::<Value>().await.unwrap()["id"], again["id"]);

    // ...but a different run option, a different scene, or no `reuse` gives a new job.
    let r = post(
        scene_json.clone(),
        json!({ "endTime": 0.15, "outputInterval": 0.05 }),
        "?reuse=completed",
    )
    .await;
    assert_eq!(r.status(), StatusCode::CREATED);
    let longer: Value = r.json().await.unwrap();
    assert_ne!(longer["id"], again["id"]);
    let mut other_scene = scene_json.clone();
    other_scene["name"] = json!("Another channel");
    let r = post(other_scene, options.clone(), "?reuse=completed").await;
    assert_eq!(r.status(), StatusCode::CREATED);
    let r = post(scene_json.clone(), options.clone(), "").await;
    assert_eq!(r.status(), StatusCode::CREATED);
    assert_ne!(r.json::<Value>().await.unwrap()["id"], again["id"]);

    // A purged job is not reused (its results are gone).
    let r = client()
        .delete(s.url(&format!(
            "/api/simulations/{}?purge=true",
            again["id"].as_str().unwrap()
        )))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());
    let r = post(scene_json.clone(), options.clone(), "?reuse=completed").await;
    // The newer run without `reuse` is the latest completed one or still running; either way it
    // is a job of this scene, not the purged one.
    assert!(matches!(r.status(), StatusCode::OK | StatusCode::CREATED));
    assert_ne!(r.json::<Value>().await.unwrap()["id"], again["id"]);

    // An unknown value is refused.
    let r = post(scene_json, options, "?reuse=always").await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"]["code"],
        "request.reuse"
    );
    s.stop().await;
}
