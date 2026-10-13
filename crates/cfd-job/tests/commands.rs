//! The command loop on the CPU backend: cancel latency, pause/resume, snapshots, unsupported
//! commands, observers, and results that do not depend on the batch size.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cfd_core::examples::{self, Canvas};
use cfd_core::scene::CellType;
use cfd_core::Scene;
use cfd_engines::{Engine, EngineOptions, SolverChoice};
use cfd_job::{
    Command, FrameEvent, JobContext, JobObserver, NoObserver, Outcome, Progress, ResultDir,
    RunOptions, Status,
};

/// A 64 × 18 channel (walls, uniform inlet, pressure outlet) in the units of the `channel`
/// example (dx = 1 mm, dt = 1 ms).
fn small_channel() -> Scene {
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

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfd-job-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn cpu() -> SolverChoice {
    SolverChoice::new(Engine::Cpu, &EngineOptions::default())
}

#[derive(Debug, Clone, PartialEq)]
enum Ev {
    Progress(u64),
    Paused(bool),
    Frame { index: Option<u32>, stored: bool },
    Log(String),
    Finished(Status),
}

/// Records every event and forwards it on a channel, so a test can wait for one.
struct Recorder {
    events: Mutex<Vec<Ev>>,
    tx: Mutex<Sender<Ev>>,
    frames: bool,
}

impl Recorder {
    fn new(frames: bool) -> (Arc<Self>, Receiver<Ev>) {
        let (tx, rx) = mpsc::channel();
        let r = Arc::new(Self {
            events: Mutex::default(),
            tx: Mutex::new(tx),
            frames,
        });
        (r, rx)
    }

    fn push(&self, e: Ev) {
        self.events.lock().unwrap().push(e.clone());
        let _ = self.tx.lock().unwrap().send(e);
    }

    fn events(&self) -> Vec<Ev> {
        self.events.lock().unwrap().clone()
    }
}

impl JobObserver for Recorder {
    fn progress(&self, p: &Progress) {
        self.push(Ev::Progress(p.step));
    }
    fn paused(&self, paused: bool) {
        self.push(Ev::Paused(paused));
    }
    fn wants_frames(&self) -> bool {
        self.frames
    }
    fn frame(&self, f: &FrameEvent) {
        self.push(Ev::Frame {
            index: f.index,
            stored: f.stored,
        });
    }
    fn log(&self, message: &str) {
        self.push(Ev::Log(message.to_string()));
    }
    fn finished(&self, o: &Outcome) {
        self.push(Ev::Finished(o.status));
    }
}

/// Waits (at most 30 s) for the first event matching `f`.
fn wait_for(rx: &Receiver<Ev>, f: impl Fn(&Ev) -> bool) -> Ev {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let e = rx.recv_timeout(left).expect("event within 30 s");
        if f(&e) {
            return e;
        }
    }
}

/// Starts a run on its own thread (as the server does) and returns the command sender and the
/// thread's handle.
fn spawn(
    scene: Scene,
    opts: RunOptions,
    dir: &Path,
    observer: Arc<dyn JobObserver>,
) -> (
    Sender<Command>,
    std::thread::JoinHandle<anyhow::Result<Outcome>>,
) {
    let prepared = cfd_job::prepare(scene.to_json_pretty(), opts).unwrap();
    let dir = ResultDir::create(dir, false).unwrap();
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let job = JobContext::new("test", "cfd-job tests");
        cfd_job::run(prepared, &cpu(), &dir, &job, observer, &rx)
    });
    (tx, handle)
}

fn meta(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap()
}

#[test]
fn cancel_stops_a_long_run_within_a_second() {
    let dir = temp_dir("cancel");
    // The full-size channel example (400 × 100) for 100 s of flow: minutes of work.
    let opts = RunOptions {
        end_time: Some(100.0),
        output_interval: Some(0.05),
        ..Default::default()
    };
    let (rec, events) = Recorder::new(false);
    let (tx, handle) = spawn(examples::channel(), opts, &dir, rec.clone());
    wait_for(&events, |e| matches!(e, Ev::Progress(s) if *s > 100));
    let sent = Instant::now();
    tx.send(Command::Cancel).unwrap();
    let outcome = handle.join().unwrap().unwrap();
    let latency = sent.elapsed();
    assert_eq!(outcome.status, Status::Cancelled);
    assert!(latency < Duration::from_secs(1), "cancelled in {latency:?}");
    eprintln!("cancel latency: {latency:?} at step {}", outcome.steps_done);
    assert!(outcome.steps_done < 100_000);

    let m = meta(&dir);
    assert_eq!(m["run"]["status"], "cancelled");
    assert_eq!(m["run"]["stepsDone"], outcome.steps_done);
    // Every frame counted in meta.json is on disk.
    let frames = m["run"]["frames"].as_u64().unwrap() as usize;
    assert_eq!(
        std::fs::read_dir(dir.join("frames")).unwrap().count(),
        frames
    );
    assert_eq!(rec.events().last(), Some(&Ev::Finished(Status::Cancelled)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pause_holds_the_solver_until_resume() {
    let dir = temp_dir("pause");
    let opts = RunOptions {
        end_time: Some(3.0),
        output_interval: Some(0.5),
        ..Default::default()
    };
    let (rec, events) = Recorder::new(false);
    let (tx, handle) = spawn(examples::channel(), opts, &dir, rec.clone());
    wait_for(&events, |e| matches!(e, Ev::Progress(_)));
    tx.send(Command::Pause).unwrap();
    wait_for(&events, |e| *e == Ev::Paused(true));
    // (A frame queued before the pause may still reach the observer from the output thread.)
    std::thread::sleep(Duration::from_millis(100));
    let before = rec.events().len();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(rec.events().len(), before, "no events while paused");

    // A snapshot is taken while paused, and changing the scene is not supported yet.
    tx.send(Command::Snapshot).unwrap();
    let snap = wait_for(&events, |e| matches!(e, Ev::Frame { index: None, .. }));
    assert_eq!(
        snap,
        Ev::Frame {
            index: None,
            stored: false
        }
    );
    tx.send(Command::SetParam(cfd_job::ParamChange {
        name: "inletVelocity".into(),
        value: 1.0,
    }))
    .unwrap();
    let log = wait_for(&events, |e| matches!(e, Ev::Log(_)));
    assert!(matches!(log, Ev::Log(m) if m.contains("not supported")));

    tx.send(Command::Resume).unwrap();
    wait_for(&events, |e| *e == Ev::Paused(false));
    let outcome = handle.join().unwrap().unwrap();
    assert_eq!(outcome.status, Status::Completed);
    assert_eq!(outcome.steps_done, 3000);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_paused_job_whose_controller_is_gone_is_cancelled() {
    let dir = temp_dir("orphan");
    let opts = RunOptions {
        end_time: Some(100.0),
        ..Default::default()
    };
    let (rec, events) = Recorder::new(false);
    let (tx, handle) = spawn(examples::channel(), opts, &dir, rec);
    tx.send(Command::Pause).unwrap();
    wait_for(&events, |e| *e == Ev::Paused(true));
    drop(tx);
    assert_eq!(handle.join().unwrap().unwrap().status, Status::Cancelled);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn observers_get_every_frame_stored_or_not() {
    // Stored: one event per frame file, in order, each after the file was written.
    let dir = temp_dir("frames-stored");
    let opts = RunOptions {
        end_time: Some(0.2),
        output_interval: Some(0.05),
        ..Default::default()
    };
    let (rec, _events) = Recorder::new(false);
    let (_tx, handle) = spawn(small_channel(), opts.clone(), &dir, rec.clone());
    assert_eq!(handle.join().unwrap().unwrap().frames, 5);
    let frames: Vec<Ev> = rec
        .events()
        .into_iter()
        .filter(|e| matches!(e, Ev::Frame { .. }))
        .collect();
    let expected: Vec<Ev> = (0..5)
        .map(|i| Ev::Frame {
            index: Some(i),
            stored: true,
        })
        .collect();
    assert_eq!(frames, expected);
    assert_eq!(std::fs::read_dir(dir.join("frames")).unwrap().count(), 5);

    // Not stored, but an observer wants them (the server streaming a run without frames).
    let dir2 = temp_dir("frames-streamed");
    let opts = RunOptions {
        write_frames: false,
        ..opts
    };
    let (rec, _events) = Recorder::new(true);
    let (_tx, handle) = spawn(small_channel(), opts, &dir2, rec.clone());
    handle.join().unwrap().unwrap();
    let streamed = rec
        .events()
        .into_iter()
        .filter(|e| matches!(e, Ev::Frame { stored: false, .. }))
        .count();
    assert_eq!(streamed, 5);
    assert!(!dir2.join("frames").exists());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

/// Batches end at every sample/output event whatever their size, so one-step batches and
/// 50 ms batches write byte-identical results.
#[test]
fn results_do_not_depend_on_the_batch_size() {
    let opts = RunOptions {
        end_time: Some(0.3),
        output_interval: Some(0.1),
        ..Default::default()
    };
    let mut dirs = Vec::new();
    for (name, batch) in [
        ("one-step", Duration::ZERO),
        ("timed", Duration::from_millis(50)),
    ] {
        let dir = temp_dir(&format!("batch-{name}"));
        let prepared = cfd_job::prepare(small_channel().to_json_pretty(), opts.clone()).unwrap();
        let rd = ResultDir::create(&dir, false).unwrap();
        let job = JobContext {
            batch_time: batch,
            ..JobContext::new("batch", "cfd-job tests")
        };
        let (_, rx) = mpsc::channel();
        let o = cfd_job::run(prepared, &cpu(), &rd, &job, Arc::new(NoObserver), &rx).unwrap();
        assert_eq!(o.status, Status::Completed);
        dirs.push(dir);
    }
    for file in [
        "probes.csv",
        "forces.csv",
        "diagnostics.csv",
        "frames/000003.bin",
    ] {
        let a = std::fs::read(dirs[0].join(file)).unwrap();
        let b = std::fs::read(dirs[1].join(file)).unwrap();
        assert!(a == b, "{file} differs");
    }
    for d in dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// Past `max_stored_bytes` the run goes on without storing frames, and says so.
#[test]
fn storing_stops_at_the_size_limit() {
    let opts = RunOptions {
        end_time: Some(0.2),
        output_interval: Some(0.05),
        ..Default::default()
    };
    let full = temp_dir("limit-full");
    let (_tx, handle) = spawn(small_channel(), opts.clone(), &full, Arc::new(NoObserver));
    handle.join().unwrap().unwrap();
    let size = |i: u32| {
        std::fs::metadata(full.join(format!("frames/{i:06}.bin")))
            .unwrap()
            .len()
    };

    let dir = temp_dir("limit");
    let opts = RunOptions {
        // Exactly the first two frames (frame 0, the fluid at rest, compresses to almost nothing).
        max_stored_bytes: Some(size(0) + size(1)),
        ..opts
    };
    let (rec, _events) = Recorder::new(false);
    let (_tx, handle) = spawn(small_channel(), opts, &dir, rec.clone());
    let outcome = handle.join().unwrap().unwrap();
    assert_eq!(outcome.status, Status::Completed);
    assert_eq!(outcome.frames, 5);
    assert_eq!(outcome.frames_stored, 2);
    assert_eq!(std::fs::read_dir(dir.join("frames")).unwrap().count(), 2);
    let m = meta(&dir);
    assert_eq!(m["run"]["frames"], 5);
    assert_eq!(m["run"]["framesStored"], 2);
    assert!(m["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["code"] == "storage.limit"));
    assert!(rec
        .events()
        .iter()
        .any(|e| matches!(e, Ev::Log(m) if m.contains("size limit"))));
    let _ = std::fs::remove_dir_all(&full);
    let _ = std::fs::remove_dir_all(&dir);
}
