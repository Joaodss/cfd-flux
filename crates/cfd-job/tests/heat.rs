//! `heat.csv`: written by the output thread from the stored frames' temperature, so the rows are
//! a function of the files on disk (and of nothing the engine does), and only thermal runs with
//! stored frames have it.

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;

use cfd_core::examples;
use cfd_core::heat::HeatWalls;
use cfd_engines::{Engine, EngineOptions, SolverChoice};
use cfd_io::frame::Frame;
use cfd_job::{JobContext, NoObserver, ResultDir, RunOptions};

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfd-job-heat-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn run(scene: &cfd_core::Scene, dir: &PathBuf, opts: RunOptions) {
    let prepared = cfd_job::prepare(scene.to_json_pretty(), opts).unwrap();
    let rd = ResultDir::create(dir, false).unwrap();
    let (_tx, rx) = mpsc::channel();
    let job = JobContext::new("heat", "cfd-job tests");
    let choice = SolverChoice::new(Engine::Cpu, &EngineOptions::default());
    cfd_job::run(prepared, &choice, &rd, &job, Arc::new(NoObserver), &rx).unwrap();
}

#[test]
fn heat_rows_are_measured_on_the_stored_frames() {
    let scene = examples::quick("heated-cavity-ra1e5").unwrap();
    let dir = temp_dir("rows");
    run(
        &scene,
        &dir,
        RunOptions {
            end_time: Some(1.0),
            output_interval: Some(0.25),
            ..Default::default()
        },
    );
    let text = std::fs::read_to_string(dir.join("heat.csv")).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "step,time_s,Hot_wall.q_W_m,Hot_wall.length_m,Cold_wall.q_W_m,Cold_wall.length_m"
    );
    let rows: Vec<Vec<f64>> = lines
        .map(|l| l.split(',').map(|v| v.parse().unwrap()).collect())
        .collect();
    let frames = std::fs::read_dir(dir.join("frames")).unwrap().count();
    assert!(frames >= 4);
    assert_eq!(rows.len(), frames, "a row per stored frame");

    // Recompute every row from the frame file alone: identical, bit for bit.
    let walls = HeatWalls::new(&scene).unwrap();
    for (i, row) in rows.iter().enumerate() {
        let bytes = std::fs::read(dir.join(format!("frames/{i:06}.bin"))).unwrap();
        let frame = Frame::decode(&bytes).unwrap();
        assert_eq!((row[0] as u64, row[1]), (frame.step, frame.sim_time));
        let temperature = frame
            .fields
            .iter()
            .find(|f| f.desc.name == "temperature")
            .expect("the cavity stores its temperature");
        let mut expected = vec![frame.step as f64, frame.sim_time];
        for h in walls.measure(&temperature.values) {
            expected.extend([h.q_w_per_m, h.length_m]);
        }
        assert_eq!(*row, expected, "frame {i}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_thermal_runs_with_stored_frames_have_heat() {
    let cavity = examples::quick("heated-cavity-ra1e5").unwrap();
    let short = RunOptions {
        end_time: Some(0.2),
        output_interval: Some(0.1),
        ..Default::default()
    };
    let dir = temp_dir("no-frames");
    run(
        &cavity,
        &dir,
        RunOptions {
            write_frames: false,
            ..short.clone()
        },
    );
    assert!(!dir.join("heat.csv").exists());
    let _ = std::fs::remove_dir_all(dir);

    let dir = temp_dir("isothermal");
    run(&examples::quick("channel").unwrap(), &dir, short);
    assert!(!dir.join("heat.csv").exists());
    let _ = std::fs::remove_dir_all(dir);
}

/// `meta.json` says there is a `heat.csv` and gives what the Nusselt number needs.
#[test]
fn meta_json_announces_the_heat_series() {
    let scene = examples::quick("heated-cavity-ra1e5").unwrap();
    let dir = temp_dir("meta");
    run(
        &scene,
        &dir,
        RunOptions {
            end_time: Some(0.1),
            output_interval: Some(0.05),
            ..Default::default()
        },
    );
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["run"]["heat"], true);
    let reference = &meta["dimensionless"]["reference"];
    assert_eq!(
        reference["conductivity"].as_f64(),
        scene.fluids[0].thermal_conductivity
    );
    assert!(reference["temperatureDifference"].as_f64().unwrap() > 0.0);
    let _ = std::fs::remove_dir_all(dir);
    // Not announced for a run without thermal walls.
    let dir = temp_dir("meta-channel");
    run(
        &examples::quick("channel").unwrap(),
        &dir,
        RunOptions {
            end_time: Some(0.1),
            ..Default::default()
        },
    );
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    assert!(meta["run"].get("heat").is_none());
    assert!(meta["dimensionless"]["reference"]
        .get("conductivity")
        .is_none());
    let _ = std::fs::remove_dir_all(dir);
}
