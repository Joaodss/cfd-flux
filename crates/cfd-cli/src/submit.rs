//! `cfd-cli submit`: submits a scene to a running `cfd-server`, follows its stream and prints
//! what arrives (frames per second, time to the first frame, progress), optionally cancelling the
//! job after a while to measure how fast a cancel takes effect. The Phase 3 acceptance client, and
//! a debugging tool for the API.
//!
//! The rest of the CLI is synchronous; this command needs to wait on several things at once (the
//! socket, a timer, Ctrl+C), so it runs on a small async runtime of its own (`block_on`, like
//! calling `.GetAwaiter().GetResult()` on an `async Task` at the top of a C# `Main`).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cfd_io::frame::Frame;
use clap::Args;
use futures_util::StreamExt;
use serde_json::value::RawValue;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message;

use crate::backend::BackendArg;

#[derive(Debug, Args)]
pub struct SubmitArgs {
    /// Scene file (JSON).
    pub scene: PathBuf,
    /// Server URL.
    #[arg(long, default_value = "http://127.0.0.1:7878")]
    pub server: String,
    /// Engine (default: the scene's preference).
    #[arg(long, value_enum)]
    pub backend: Option<BackendArg>,
    /// Lattice velocity the characteristic velocity maps to.
    #[arg(long)]
    pub lattice_velocity: Option<f64>,
    /// Simulated time in seconds (overrides the scene's `run.endTime`).
    #[arg(long, value_name = "SECONDS")]
    pub end_time: Option<f64>,
    /// Time between frames in seconds (overrides `run.outputInterval`).
    #[arg(long, value_name = "SECONDS")]
    pub output_interval: Option<f64>,
    /// Time between probe/force/diagnostic samples in seconds.
    #[arg(long, value_name = "SECONDS")]
    pub sample_interval: Option<f64>,
    /// Do not store frames on the server (stream them only).
    #[arg(long)]
    pub no_store: bool,
    /// Fields to stream (default: every field of the frames).
    #[arg(long, value_delimiter = ',')]
    pub fields: Option<Vec<String>>,
    /// Longest side of streamed frames, in cells (default: the server's).
    #[arg(long)]
    pub max_size: Option<u32>,
    /// Cancel the job after streaming for this long, and report how long the cancel took.
    #[arg(long, value_name = "SECONDS")]
    pub cancel_after: Option<f64>,
    /// Only submit; print the job's id and return.
    #[arg(long, conflicts_with = "cancel_after")]
    pub detach: bool,
}

fn backend_name(b: BackendArg) -> &'static str {
    match b {
        BackendArg::Auto => "auto",
        BackendArg::Cpu => "cpu",
        BackendArg::Wgpu => "wgpu",
        BackendArg::Cuda => "cuda",
    }
}

pub fn submit(args: SubmitArgs) -> Result<ExitCode> {
    // One thread is plenty: this is all waiting on I/O.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run(args))
}

/// Frame statistics as they arrive.
#[derive(Default)]
struct Stats {
    frames: u32,
    bytes: u64,
    first: Option<Duration>,
    last: Option<Duration>,
    last_index: Option<u32>,
}

async fn run(args: SubmitArgs) -> Result<ExitCode> {
    let server = args.server.trim_end_matches('/').to_string();
    let text = std::fs::read_to_string(&args.scene)
        .with_context(|| format!("cannot read {}", args.scene.display()))?;
    // The scene goes into the request exactly as in the file (the server stores it as scene.json).
    let scene = RawValue::from_string(text)
        .with_context(|| format!("{} is not JSON", args.scene.display()))?;
    let mut options = serde_json::Map::new();
    let mut put = |k: &str, v: Option<Value>| {
        if let Some(v) = v {
            options.insert(k.into(), v);
        }
    };
    put("backend", args.backend.map(|b| json!(backend_name(b))));
    put("latticeVelocity", args.lattice_velocity.map(|v| json!(v)));
    put("endTime", args.end_time.map(|v| json!(v)));
    put("outputInterval", args.output_interval.map(|v| json!(v)));
    put("sampleInterval", args.sample_interval.map(|v| json!(v)));
    put("storeFrames", args.no_store.then_some(json!(false)));

    let http = reqwest::Client::new();
    let t0 = Instant::now();
    let r = http
        .post(format!("{server}/api/simulations"))
        .json(&json!({ "scene": scene, "options": options }))
        .send()
        .await
        .with_context(|| format!("cannot reach {server} (is cfd-server running?)"))?;
    let status = r.status();
    let body: Value = r.json().await.context("the server's answer is not JSON")?;
    if !status.is_success() {
        let e = &body["error"];
        eprintln!(
            "error: the server refused the job ({status}): [{}] {}",
            e["code"].as_str().unwrap_or("?"),
            e["message"].as_str().unwrap_or("")
        );
        for i in e["issues"].as_array().into_iter().flatten() {
            eprintln!(
                "  {} [{}] {}",
                i["severity"].as_str().unwrap_or(""),
                i["code"].as_str().unwrap_or(""),
                i["message"].as_str().unwrap_or("")
            );
        }
        return Ok(ExitCode::FAILURE);
    }
    let id = body["id"]
        .as_str()
        .context("no job id in the answer")?
        .to_string();
    let est = &body["estimate"];
    eprintln!(
        "job {id}: {} on {}{} — {} cells, {} steps, {} frames{}",
        body["status"].as_str().unwrap_or("?"),
        body["backend"].as_str().unwrap_or("?"),
        body["device"]
            .as_str()
            .map(|d| format!(" ({d})"))
            .unwrap_or_default(),
        est["cells"],
        est["steps"],
        est["frames"],
        est["seconds"]
            .as_f64()
            .map(|s| format!(", ≈ {s:.1} s of solver time"))
            .unwrap_or_default(),
    );
    for w in body["warnings"].as_array().into_iter().flatten() {
        eprintln!(
            "  warning [{}] {}",
            w["code"].as_str().unwrap_or(""),
            w["message"].as_str().unwrap_or("")
        );
    }
    for n in body["notes"].as_array().into_iter().flatten() {
        eprintln!("  note: {}", n.as_str().unwrap_or(""));
    }
    if args.detach {
        println!("{id}");
        return Ok(ExitCode::SUCCESS);
    }

    let mut query = Vec::new();
    if let Some(f) = &args.fields {
        query.push(format!("fields={}", f.join(",")));
    }
    if let Some(m) = args.max_size {
        query.push(format!("maxSize={m}"));
    }
    let ws_url = format!(
        "{}/api/simulations/{id}/stream{}{}",
        server.replacen("http", "ws", 1),
        if query.is_empty() { "" } else { "?" },
        query.join("&")
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .with_context(|| format!("cannot open {ws_url}"))?;

    let mut stats = Stats::default();
    let mut last_progress = Instant::now() - Duration::from_secs(10);
    let streaming = Instant::now();
    let cancel_at = args
        .cancel_after
        .map(|s| tokio::time::Instant::now() + Duration::from_secs_f64(s));
    let mut cancel_sent: Option<Instant> = None;
    let cancel_url = format!("{server}/api/simulations/{id}");
    let done: Value = loop {
        // Whichever comes first: a message, the cancel time, Ctrl+C.
        let msg = tokio::select! {
            m = ws.next() => m,
            _ = async { tokio::time::sleep_until(cancel_at.unwrap()).await }, if cancel_at.is_some() && cancel_sent.is_none() => {
                eprintln!("cancelling after {:.2} s of streaming", streaming.elapsed().as_secs_f64());
                cancel_sent = Some(Instant::now());
                http.delete(&cancel_url).send().await?.error_for_status()?;
                continue;
            }
            _ = tokio::signal::ctrl_c(), if cancel_sent.is_none() => {
                eprintln!("Ctrl+C: cancelling the job");
                cancel_sent = Some(Instant::now());
                http.delete(&cancel_url).send().await?;
                continue;
            }
        };
        match msg {
            Some(Ok(Message::Binary(b))) => {
                let at = t0.elapsed();
                let frame = Frame::decode(&b).context("a stream frame does not decode")?;
                if stats.first.is_none() {
                    eprintln!(
                        "  first frame {:.2} s after submitting (frame {}, step {}, {}×{}, {} kB)",
                        at.as_secs_f64(),
                        frame.frame_index,
                        frame.step,
                        frame.width,
                        frame.height,
                        b.len() / 1000
                    );
                    stats.first = Some(at);
                }
                stats.frames += 1;
                stats.bytes += b.len() as u64;
                stats.last = Some(at);
                stats.last_index = Some(frame.frame_index);
            }
            Some(Ok(Message::Text(t))) => {
                let v: Value = serde_json::from_str(&t).context("a stream message is not JSON")?;
                match v["type"].as_str() {
                    Some("hello") => {
                        let f = &v["format"];
                        if f.is_object() {
                            eprintln!(
                                "  streaming {}×{} (factor {}, {}), status {}",
                                f["width"],
                                f["height"],
                                f["factor"],
                                f["dtype"].as_str().unwrap_or(""),
                                v["job"]["status"].as_str().unwrap_or("?")
                            );
                        } else {
                            eprintln!(
                                "  streaming, status {}",
                                v["job"]["status"].as_str().unwrap_or("?")
                            );
                        }
                    }
                    Some("status") => eprintln!("  status {}", v["status"].as_str().unwrap_or("?")),
                    Some("log") => eprintln!("  server: {}", v["message"].as_str().unwrap_or("")),
                    Some("progress") if last_progress.elapsed() >= Duration::from_secs(1) => {
                        last_progress = Instant::now();
                        eprintln!(
                            "  step {}/{} ({:.0}%), t = {:.4} s, {:.0} MLUPS, max Ma = {:.3}, {} frames received",
                            v["step"], v["total"],
                            v["percent"].as_f64().unwrap_or(0.0),
                            v["simTime"].as_f64().unwrap_or(0.0),
                            v["mlups"].as_f64().unwrap_or(0.0),
                            v["maxMach"].as_f64().unwrap_or(0.0),
                            stats.frames
                        );
                    }
                    Some("done") => break v,
                    _ => {}
                }
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => bail!("the stream failed: {e}"),
            None => bail!("the server closed the stream before the job ended"),
        }
    };

    let status = done["status"].as_str().unwrap_or("?");
    if let Some(sent) = cancel_sent {
        eprintln!(
            "  cancel took effect after {:.3} s",
            sent.elapsed().as_secs_f64()
        );
    }
    let produced = done["frames"].as_u64().unwrap_or(0);
    if let (Some(first), Some(last)) = (stats.first, stats.last) {
        let span = (last - first).as_secs_f64();
        let rate = if span > 0.0 {
            (stats.frames - 1) as f64 / span
        } else {
            0.0
        };
        eprintln!(
            "  {} frames received ({} produced{}), {rate:.1} frames/s, {:.1} MB/s",
            stats.frames,
            produced,
            match produced.saturating_sub(stats.frames as u64) {
                0 => String::new(),
                n => format!(", {n} skipped by backpressure"),
            },
            if span > 0.0 {
                stats.bytes as f64 / span / 1e6
            } else {
                0.0
            }
        );
    }
    eprintln!(
        "done: {status}{} — {} steps, {} frames, results in the server's results/{id}/",
        done["reason"]
            .as_str()
            .map(|r| format!(" ({r})"))
            .unwrap_or_default(),
        done["stepsDone"],
        produced
    );
    if let Some(m) = done["message"].as_str() {
        eprintln!("  {m}");
    }
    let ok = status == "completed" || (status == "cancelled" && cancel_sent.is_some());
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
