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
