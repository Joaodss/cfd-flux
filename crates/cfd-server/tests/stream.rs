//! The WebSocket stream: hello, frames, events, backpressure, late joiners; and the Phase 3
//! acceptance scenario (submit the cylinder, receive frames in real time, cancel in < 1 s).

mod common;

use std::time::{Duration, Instant};

use cfd_core::examples;
use cfd_core::Scene;
use cfd_engines::{EngineKind, EngineOptions, Inventory};
use cfd_io::frame::{Dtype, Frame};
use common::{client, TestServer};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug)]
enum Msg {
    Text(Value),
    Frame(Frame),
    Closed,
}

async fn connect(s: &TestServer, id: &str, query: &str) -> Socket {
    let url = s.ws_url(&format!("/api/simulations/{id}/stream{query}"));
    tokio_tungstenite::connect_async(url).await.unwrap().0
}

/// The next message, at most 60 s away.
async fn next(ws: &mut Socket) -> Msg {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(60), ws.next())
            .await
            .expect("a message within 60 s");
        return match m {
            None | Some(Ok(Message::Close(_))) => Msg::Closed,
            Some(Ok(Message::Text(t))) => Msg::Text(serde_json::from_str(&t).unwrap()),
            Some(Ok(Message::Binary(b))) => Msg::Frame(Frame::decode(&b).unwrap()),
            Some(Ok(_)) => continue,
            Some(Err(e)) => panic!("websocket error: {e}"),
        };
    }
}

/// Everything until the socket closes: (texts, frames).
async fn drain(ws: &mut Socket) -> (Vec<Value>, Vec<Frame>) {
    let (mut texts, mut frames) = (Vec::new(), Vec::new());
    loop {
        match next(ws).await {
            Msg::Text(v) => texts.push(v),
            Msg::Frame(f) => frames.push(f),
            Msg::Closed => return (texts, frames),
        }
    }
}

async fn submit(s: &TestServer, scene: &Scene, options: Value) -> String {
    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "scene": scene, "options": options }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let v: Value = r.json().await.unwrap();
    v["id"].as_str().unwrap().to_string()
}

async fn job(s: &TestServer, id: &str) -> Value {
    client()
        .get(s.url(&format!("/api/simulations/{id}")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// The channel example (400 × 100): about a second of CPU work per 300 steps in a debug build.
fn channel_options(end_time: f64, output_interval: f64) -> Value {
    json!({ "endTime": end_time, "outputInterval": output_interval })
}

#[tokio::test]
async fn a_viewer_gets_hello_frames_progress_and_done() {
    let s = TestServer::start("ws-basic").await;
    let id = submit(&s, &examples::channel(), channel_options(0.3, 0.02)).await;
    let mut ws = connect(&s, &id, "").await;
    let Msg::Text(hello) = next(&mut ws).await else {
        panic!("hello first")
    };
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["job"]["id"], id.as_str());
    let (texts, frames) = drain(&mut ws).await;

    let done = texts.last().unwrap();
    assert_eq!(done["type"], "done", "{texts:?}");
    assert_eq!(done["status"], "completed");
    assert_eq!(done["frames"], 16);
    assert!(texts.iter().any(|t| t["type"] == "progress"));
    assert!(frames.len() >= 3, "{} frames", frames.len());
    // In order, f16, full size (400 × 100 fits 1024), and the last one is the final state.
    assert!(frames
        .windows(2)
        .all(|w| w[0].frame_index < w[1].frame_index));
    let last = frames.last().unwrap();
    assert_eq!((last.frame_index, last.step), (15, 300));
    assert_eq!((last.width, last.height), (400, 100));
    assert_eq!(last.job_id, id);
    assert!(last.fields.iter().all(|f| f.desc.dtype == Dtype::F16));
    let names: Vec<&str> = last.fields.iter().map(|f| f.desc.name.as_str()).collect();
    assert_eq!(names, ["velocity", "pressure", "vorticity"]);
    // Stored frames are all on disk, whatever was streamed.
    let stored = std::fs::read_dir(s.data.join(format!("results/{id}/frames")))
        .unwrap()
        .count();
    assert_eq!(stored, 16);
    s.stop().await;
}

#[tokio::test]
async fn each_viewer_gets_its_own_format() {
    let s = TestServer::start("ws-formats").await;
    let id = submit(&s, &examples::channel(), channel_options(0.2, 0.02)).await;
    let mut small = connect(&s, &id, "?fields=pressure&maxSize=100").await;
    let mut full = connect(&s, &id, "").await;
    let Msg::Text(hello) = next(&mut small).await else {
        panic!("hello first")
    };
    if !hello["format"].is_null() {
        assert_eq!(hello["format"]["factor"], 4);
        assert_eq!(hello["format"]["width"], 100);
    }
    let (_, a) = drain(&mut small).await;
    let (_, b) = drain(&mut full).await;
    let (fa, fb) = (a.last().unwrap(), b.last().unwrap());
    assert_eq!((fa.width, fa.height, fa.fields.len()), (100, 25, 1));
    assert_eq!(fa.fields[0].desc.name, "pressure");
    assert_eq!((fb.width, fb.height, fb.fields.len()), (400, 100, 3));
    assert_eq!(fa.step, fb.step);
    s.stop().await;
}

/// A viewer that stops reading never holds the job back: the job completes while the viewer
/// reads nothing; afterwards the viewer gets the latest frames (having skipped what the socket
/// could not hold) and `done`.
#[tokio::test]
async fn a_viewer_that_does_not_read_never_slows_the_job() {
    let s = TestServer::start("ws-slow").await;
    let id = submit(&s, &examples::channel(), channel_options(0.3, 0.002)).await;
    let mut ws = connect(&s, &id, "").await;
    let started = Instant::now();
    loop {
        let v = job(&s, &id).await;
        if v["status"] == "completed" {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "{v}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (texts, frames) = drain(&mut ws).await;
    let produced = 151;
    assert_eq!(texts.last().unwrap()["type"], "done");
    assert_eq!(texts.last().unwrap()["frames"], produced);
    let last = frames.last().unwrap();
    assert_eq!(last.frame_index, produced - 1, "the final frame arrives");
    assert!(frames
        .windows(2)
        .all(|w| w[0].frame_index < w[1].frame_index));
    eprintln!(
        "slow viewer: {} of {produced} frames (the rest skipped)",
        frames.len()
    );
    let stored = std::fs::read_dir(s.data.join(format!("results/{id}/frames")))
        .unwrap()
        .count();
    assert_eq!(stored as u32, produced, "every frame is stored");
    s.stop().await;
}

#[tokio::test]
async fn a_viewer_joining_a_paused_job_gets_a_frame_at_once() {
    let s = TestServer::start("ws-paused").await;
    let id = submit(&s, &examples::channel(), channel_options(100.0, 50.0)).await;
    loop {
        if job(&s, &id).await["stepsDone"].as_u64().unwrap_or(0) > 10 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    client()
        .post(s.url(&format!("/api/simulations/{id}/pause")))
        .send()
        .await
        .unwrap();
    while job(&s, &id).await["status"] != "paused" {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // A format nobody asked for yet: there is no frame of it until the job takes a snapshot.
    let mut ws = connect(&s, &id, "?maxSize=100").await;
    let Msg::Text(hello) = next(&mut ws).await else {
        panic!("hello first")
    };
    assert_eq!(hello["job"]["status"], "paused");
    assert_eq!(hello["grid"]["width"], 400);
    let t = Instant::now();
    let frame = loop {
        match next(&mut ws).await {
            Msg::Frame(f) => break f,
            Msg::Text(_) => continue,
            Msg::Closed => panic!("closed"),
        }
    };
    assert!(t.elapsed() < Duration::from_secs(2));
    assert_eq!((frame.width, frame.height), (100, 25));
    assert!(frame.step > 10);

    client()
        .delete(s.url(&format!("/api/simulations/{id}")))
        .send()
        .await
        .unwrap();
    let (texts, _) = drain(&mut ws).await;
    let done = texts.last().unwrap();
    assert_eq!(
        (&done["type"], &done["status"]),
        (&json!("done"), &json!("cancelled"))
    );
    s.stop().await;
}

#[tokio::test]
async fn unknown_and_finished_jobs() {
    let s = TestServer::start("ws-edges").await;
    let url = s.ws_url("/api/simulations/nope/stream");
    match tokio_tungstenite::connect_async(url).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(r)) => assert_eq!(r.status(), 404),
        other => panic!("expected a 404, got {other:?}"),
    }

    let id = submit(&s, &common::small_channel(), json!({ "endTime": 0.01 })).await;
    while job(&s, &id).await["status"] != "completed" {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut ws = connect(&s, &id, "").await;
    let (texts, frames) = drain(&mut ws).await;
    assert_eq!(texts[0]["type"], "hello");
    assert_eq!(texts[1]["type"], "done");
    assert_eq!(texts[1]["status"], "completed");
    assert!(frames.is_empty());

    // Client messages are ignored (reserved for interactive mode), not an error.
    let id = submit(&s, &examples::channel(), channel_options(0.1, 0.05)).await;
    let mut ws = connect(&s, &id, "").await;
    ws.send(Message::Text("{\"type\":\"patch\"}".into()))
        .await
        .unwrap();
    let (texts, _) = drain(&mut ws).await;
    assert_eq!(texts.last().unwrap()["status"], "completed");
    s.stop().await;
}

/// A run's file through the API, as text.
async fn file(s: &TestServer, id: &str, name: &str) -> String {
    let r = client()
        .get(s.url(&format!("/api/simulations/{id}/files/{name}")))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{name}: {}", r.status());
    r.text().await.unwrap()
}

/// `hello.series` names the columns and the `series` messages carry the rows: together, from
/// the moment the viewer joined, exactly the end of each CSV file (Phase 5 design proposal §7.1).
/// The last rows come before `done`, and every `series` message right before a `progress`.
#[tokio::test]
async fn series_rows_reach_viewers_and_end_the_files() {
    let s = TestServer::start("ws-series").await;
    let id = submit(&s, &examples::channel(), channel_options(0.3, 0.02)).await;
    // Wait until it runs, so `hello` has the columns.
    while job(&s, &id).await["stepsDone"].as_u64().unwrap_or(0) == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let mut ws = connect(&s, &id, "").await;
    let Msg::Text(hello) = next(&mut ws).await else {
        panic!("hello first")
    };
    let (texts, _) = drain(&mut ws).await;
    let kinds: Vec<&str> = texts.iter().map(|t| t["type"].as_str().unwrap()).collect();
    assert_eq!(kinds[kinds.len() - 2..], ["series", "done"], "{kinds:?}");
    for w in kinds.windows(2) {
        if w[0] == "series" {
            assert!(matches!(w[1], "progress" | "done"), "{kinds:?}");
        }
    }

    for name in ["forces", "probes", "diagnostics"] {
        let text = file(&s, &id, &format!("{name}.csv")).await;
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        assert_eq!(hello["series"][name], json!(header), "{name} columns");
        let on_disk: Vec<Vec<f64>> = lines
            .map(|l| l.split(',').map(|v| v.parse().unwrap()).collect())
            .collect();
        let streamed: Vec<Vec<f64>> = texts
            .iter()
            .filter(|t| t["type"] == "series")
            .flat_map(|t| t["rows"][name].as_array().cloned().unwrap_or_default())
            .map(|r| {
                r.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_f64().unwrap())
                    .collect()
            })
            .collect();
        assert!(!streamed.is_empty(), "{name}: rows streamed");
        let tail = &on_disk[on_disk.len() - streamed.len()..];
        // JSON numbers parse back to within an ulp (serde_json's fast float parsing).
        for (a, b) in tail.iter().flatten().zip(streamed.iter().flatten()) {
            assert!((a - b).abs() <= 1e-15 * a.abs(), "{name}: {a} vs {b}");
        }
        assert_eq!(tail.last().unwrap()[0], 300.0, "{name}: the last step");
    }
    s.stop().await;
}

/// `heat.csv` (Phase 5 design proposal §7.3) reaches viewers like the other series: its columns in
/// `hello.series.heat`, a row per stored frame in the `series` messages, and every row a viewer
/// has been sent is already on disk when it arrives. Only thermal runs with stored frames get it.
#[tokio::test]
async fn heat_rows_are_flushed_before_they_are_sent() {
    let s = TestServer::start("ws-heat").await;
    // The quick heated cavity (66 × 66 cells): 5 frames, a few hundred steps.
    let cavity = examples::quick("heated-cavity-ra1e5").unwrap();
    let options = json!({ "endTime": 2.0, "outputInterval": 0.5 });
    let id = submit(&s, &cavity, options.clone()).await;
    while job(&s, &id).await["stepsDone"].as_u64().unwrap_or(0) == 0 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let mut ws = connect(&s, &id, "").await;
    let Msg::Text(hello) = next(&mut ws).await else {
        panic!("hello first")
    };
    let columns = json!([
        "step",
        "time_s",
        "Hot_wall.q_W_m",
        "Hot_wall.length_m",
        "Cold_wall.q_W_m",
        "Cold_wall.length_m"
    ]);
    assert_eq!(hello["series"]["heat"], columns);

    let mut streamed: Vec<Vec<f64>> = Vec::new();
    loop {
        match next(&mut ws).await {
            Msg::Text(t) if t["type"] == "series" => {
                let rows: Vec<Vec<f64>> = t["rows"]["heat"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .map(|r| {
                        r.as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_f64().unwrap())
                            .collect()
                    })
                    .collect();
                if let Some(last) = rows.last() {
                    // On disk already: the file ends with (at least) the rows just received.
                    let text = file(&s, &id, "heat.csv").await;
                    let last_on_disk: f64 = text
                        .lines()
                        .last()
                        .unwrap()
                        .split(',')
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap();
                    assert!(last_on_disk >= last[0], "{last_on_disk} < {}", last[0]);
                }
                streamed.extend(rows);
            }
            Msg::Closed => break,
            _ => {}
        }
    }

    let text = file(&s, &id, "heat.csv").await;
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        columns
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap())
            .collect::<Vec<_>>()
            .join(",")
    );
    let on_disk: Vec<Vec<f64>> = lines
        .map(|l| l.split(',').map(|v| v.parse().unwrap()).collect())
        .collect();
    // A row per stored frame, at the frame's time.
    assert_eq!(on_disk.len(), 5);
    let times: Vec<f64> = on_disk.iter().map(|r| r[1]).collect();
    for (t, expected) in times.iter().zip([0.0, 0.5, 1.0, 1.5, 2.0]) {
        assert!((t - expected).abs() < 1e-3, "{times:?}");
    }
    // The hot wall gives heat to the fluid, the cold wall takes it; each is 64 cells long.
    let length = 64.0 * cavity.grid.cell_size;
    for r in &on_disk {
        assert!(r[2] > 0.0 && r[4] < 0.0, "{r:?}");
        assert!((r[3] - length).abs() < 1e-12 && (r[5] - length).abs() < 1e-12);
    }
    // The stream's rows are the end of the file.
    assert!(!streamed.is_empty());
    let tail = &on_disk[on_disk.len() - streamed.len()..];
    for (a, b) in tail.iter().flatten().zip(streamed.iter().flatten()) {
        assert!((a - b).abs() <= 1e-15 * a.abs(), "{a} vs {b}");
    }

    // No stored frames, no heat; and a run without thermal walls has none either.
    let id = submit(
        &s,
        &cavity,
        json!({ "endTime": 0.2, "outputInterval": 0.1, "storeFrames": false }),
    )
    .await;
    wait_done(&s, &id).await;
    assert_eq!(heat_status(&s, &id).await, 404);
    let id = submit(&s, &examples::channel(), channel_options(0.1, 0.05)).await;
    wait_done(&s, &id).await;
    assert_eq!(heat_status(&s, &id).await, 404);
    let mut ws = connect(&s, &id, "").await;
    let Msg::Text(hello) = next(&mut ws).await else {
        panic!("hello first")
    };
    assert!(hello["series"].get("heat").is_none() || hello["series"]["heat"].is_null());
    s.stop().await;
}

async fn wait_done(s: &TestServer, id: &str) {
    while job(s, id).await["status"] != "completed" {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn heat_status(s: &TestServer, id: &str) -> u16 {
    client()
        .get(s.url(&format!("/api/simulations/{id}/files/heat.csv")))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// Stored frames fetched in a stream format (`fields`, `maxSize`) equal the stream's frames of
/// the same index byte for byte (on the CPU engine, where both come from the same f32 values), so
/// the viewer can mix live and stored frames (Phase 5 design proposal §3.1).
#[tokio::test]
async fn stored_frames_in_a_stream_format_equal_stream_frames() {
    let s = TestServer::start("ws-stored-format").await;
    let id = submit(&s, &examples::channel(), channel_options(0.2, 0.02)).await;
    // 400 × 100 cells at most 128 on a side: factor 4, 100 × 25 blocks.
    let mut ws = connect(&s, &id, "?maxSize=128").await;
    let mut streamed: Vec<Vec<u8>> = Vec::new();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(60), ws.next())
            .await
            .expect("a message within 60 s");
        match m {
            None | Some(Ok(Message::Close(_))) => break,
            Some(Ok(Message::Binary(b))) => streamed.push(b.to_vec()),
            Some(Ok(_)) => {}
            Some(Err(e)) => panic!("websocket error: {e}"),
        }
    }
    assert!(!streamed.is_empty());
    let get = |query: String| {
        let url = s.url(&query);
        async move {
            let r = client().get(url).send().await.unwrap();
            assert_eq!(r.status(), 200);
            r.bytes().await.unwrap().to_vec()
        }
    };
    for bytes in &streamed {
        let h = Frame::decode_header(bytes).unwrap();
        assert_eq!((h.width, h.height), (100, 25));
        let stored = get(format!(
            "/api/simulations/{id}/frames/{}?dtype=f16&maxSize=128",
            h.frame_index
        ))
        .await;
        assert!(stored == *bytes, "frame {} differs", h.frame_index);
    }
    // A subset of the fields, in frame order whatever the order asked for, full size, f32.
    let f = Frame::decode(
        &get(format!(
            "/api/simulations/{id}/frames/3?fields=vorticity,velocity"
        ))
        .await,
    )
    .unwrap();
    let names: Vec<&str> = f.fields.iter().map(|f| f.desc.name.as_str()).collect();
    assert_eq!(names, ["velocity", "vorticity"]);
    assert_eq!((f.width, f.height), (400, 100));
    assert!(f.fields.iter().all(|f| f.desc.dtype == Dtype::F32));
    s.stop().await;
}

/// Whether an engine can run here; GPU engines are skipped (with a message) unless
/// `CFD_REQUIRE_GPU` / `CFD_REQUIRE_CUDA` asks for them.
fn available(kind: EngineKind) -> bool {
    let inv = Inventory::probe_only(
        &[kind],
        &EngineOptions {
            allow_software: true,
            ..Default::default()
        },
    );
    let info = inv.info(kind).unwrap();
    if info.available {
        return true;
    }
    let required = match kind {
        EngineKind::Cuda => std::env::var_os("CFD_REQUIRE_CUDA").is_some(),
        EngineKind::Wgpu => std::env::var_os("CFD_REQUIRE_GPU").is_some(),
        EngineKind::Cpu => true,
    };
    assert!(
        !required,
        "{kind} is required but unavailable: {:?}",
        info.reason
    );
    eprintln!("skipping {kind}: {:?}", info.reason);
    false
}

/// The cancel-latency bound: < 1 s is the acceptance for a release build (30–46 ms measured). An
/// unoptimised build encodes and writes the frames still queued at the cancel much more slowly,
/// and on a small shared CI runner that flush alone can pass a second.
const CANCEL_BOUND: Duration = if cfg!(debug_assertions) {
    Duration::from_secs(5)
} else {
    Duration::from_secs(1)
};

/// Phase 3 acceptance: a client submits the cylinder, receives frames in real time, and a cancel
/// ends the job in less than a second (in a release build; debug builds get more room, see
/// `CANCEL_BOUND`).
async fn acceptance(kind: EngineKind) {
    if !available(kind) {
        return;
    }
    let s = TestServer::with(&format!("acceptance-{kind}"), &[kind], |_| {}).await;
    let scene = examples::cylinder_re100();
    let t0 = Instant::now();
    let r = client()
        .post(s.url("/api/simulations"))
        .json(&json!({ "scene": scene, "options": { "backend": kind.name(), "outputInterval": 0.01 } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut ws = connect(&s, &id, "").await;
    let mut frames = Vec::new();
    while frames.len() < 5 {
        match next(&mut ws).await {
            Msg::Frame(f) => frames.push((t0.elapsed(), f)),
            Msg::Text(v) => assert_ne!(v["type"], "done", "{v}"),
            Msg::Closed => panic!("closed early"),
        }
    }
    let first = frames[0].0;
    let rate = (frames.len() - 1) as f64 / (frames[4].0 - first).as_secs_f64();
    let t = Instant::now();
    let r = client()
        .delete(s.url(&format!("/api/simulations/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    let done = loop {
        match next(&mut ws).await {
            Msg::Text(v) if v["type"] == "done" => break v,
            Msg::Closed => panic!("closed before done"),
            _ => {}
        }
    };
    let cancel = t.elapsed();
    eprintln!(
        "{kind}: first frame after {first:?}, {rate:.1} frames/s, cancelled in {cancel:?} at step {}",
        done["stepsDone"]
    );
    assert_eq!(done["status"], "cancelled");
    assert!(cancel < CANCEL_BOUND, "cancel took {cancel:?}");
    let f = &frames[4].1;
    assert_eq!((f.width, f.height), (scene.grid.width, scene.grid.height));
    assert!(frames.windows(2).all(|w| w[0].1.step < w[1].1.step));
    s.stop().await;
}

#[tokio::test]
async fn acceptance_cylinder_cpu() {
    acceptance(EngineKind::Cpu).await;
}

#[tokio::test]
async fn acceptance_cylinder_wgpu() {
    acceptance(EngineKind::Wgpu).await;
}

#[tokio::test]
async fn acceptance_cylinder_cuda() {
    acceptance(EngineKind::Cuda).await;
}
