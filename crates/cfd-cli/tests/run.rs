//! End-to-end tests of `cfd-cli run` on small scenes (the binary is built by cargo for tests).

use std::path::{Path, PathBuf};
use std::process::Command;

use cfd_core::examples::{self, Canvas};
use cfd_core::scene::CellType;
use cfd_core::Scene;

/// A 64 × 18 channel (walls, uniform inlet, pressure outlet) with a probe, in the units of the
/// `channel` example (dx = 1 mm, dt = 1 ms at the default lattice velocity).
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
    let d = std::env::temp_dir().join(format!("cfd-cli-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Backend arguments; `None` for wgpu when the machine has no adapter (the test is skipped
/// unless `CFD_REQUIRE_GPU` is set).
fn backend_args(backend: &str) -> Option<Vec<&'static str>> {
    match backend {
        "cpu" => Some(vec!["--backend", "cpu"]),
        "cuda" => match cuda_available() {
            Ok(()) => Some(vec!["--backend", "cuda"]),
            Err(e) => {
                assert!(
                    std::env::var_os("CFD_REQUIRE_CUDA").is_none(),
                    "CFD_REQUIRE_CUDA is set but CUDA is unavailable: {e}"
                );
                eprintln!("skipping: {e}");
                None
            }
        },
        _ if cfd_gpu::adapters().is_empty() => {
            assert!(
                std::env::var_os("CFD_REQUIRE_GPU").is_none(),
                "CFD_REQUIRE_GPU is set but there is no wgpu adapter"
            );
            eprintln!("skipping: no wgpu adapter");
            None
        }
        _ => Some(vec!["--backend", "wgpu", "--allow-software"]),
    }
}

fn run(scene: &Scene, dir: &Path, extra: &[&str]) -> std::process::Output {
    run_env(scene, dir, extra, &[])
}

fn run_env(
    scene: &Scene,
    dir: &Path,
    extra: &[&str],
    env: &[(&str, &str)],
) -> std::process::Output {
    let scene_path = dir.join("scene-in.json");
    std::fs::write(&scene_path, scene.to_json_pretty()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_cfd-cli"))
        .arg("run")
        .arg(&scene_path)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--threads", "2"])
        .args(extra)
        .envs(env.iter().copied())
        .output()
        .expect("cfd-cli runs")
}

fn meta(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("out/meta.json")).unwrap()).unwrap()
}

fn count(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| d.count())
}

#[test]
fn run_writes_frames_series_images_and_metadata_cpu() {
    writes_outputs("cpu");
}

#[test]
fn run_writes_frames_series_images_and_metadata_wgpu() {
    writes_outputs("wgpu");
}

#[test]
fn run_writes_frames_series_images_and_metadata_cuda() {
    writes_outputs("cuda");
}

fn writes_outputs(backend: &str) {
    let Some(mut args) = backend_args(backend) else {
        return;
    };
    let dir = temp_dir(&format!("ok-{backend}"));
    args.extend(["--end-time", "0.2", "--output-interval", "0.05"]);
    let out = run(&small_channel(), &dir, &args);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let m = meta(&dir);
    assert_eq!(m["run"]["status"], "completed");
    assert_eq!(m["solver"]["backend"], backend);
    assert_eq!(m["run"]["stepsDone"], 200);
    assert_eq!(m["run"]["frames"], 5);
    assert_eq!(m["units"]["dt"], 0.001);
    assert!(m["dimensionless"]["reynolds"].as_f64().unwrap() > 0.0);
    assert!(m["timings"]["mlups"].as_f64().unwrap() > 0.0);

    let o = dir.join("out");
    assert_eq!(count(&o.join("frames")), 5);
    assert_eq!(count(&o.join("png/velocity")), 5);
    assert_eq!(count(&o.join("png/vorticity")), 5);
    assert!(o.join("png/ranges.json").exists());
    assert_eq!(
        std::fs::read_to_string(o.join("scene.json")).unwrap(),
        small_channel().to_json_pretty()
    );

    // Frames decode and carry the scene's output fields plus the rendered ones.
    let frame =
        cfd_io::frame::Frame::decode(&std::fs::read(o.join("frames/000004.bin")).unwrap()).unwrap();
    assert_eq!(frame.step, 200);
    let names: Vec<&str> = frame.fields.iter().map(|f| f.desc.name.as_str()).collect();
    assert_eq!(names, ["velocity", "pressure", "vorticity"]);

    // Samples every output interval / 10 = 5 steps: 41 rows plus the header.
    let probes = std::fs::read_to_string(o.join("probes.csv")).unwrap();
    assert_eq!(probes.lines().count(), 42);
    assert!(probes.starts_with("step,time_s,Centre.ux_m_s,"));
    let last: Vec<f64> = probes
        .lines()
        .last()
        .unwrap()
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    assert_eq!(last[0], 200.0);
    assert!(last[2] > 0.0, "flow reaches the probe");
    let forces = std::fs::read_to_string(o.join("forces.csv")).unwrap();
    assert!(forces.lines().next().unwrap().contains("Inlet.fx_N_m"));
    assert_eq!(forces.lines().count(), 42);
    // Gauge forces at step 0 (fluid at rest): nothing on the walls or the outlet; only the
    // moving inlet pushes the fluid (columns: edges, Walls, Inlet, Outlet).
    let first: Vec<f64> = forces
        .lines()
        .nth(1)
        .unwrap()
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    let walls_and_outlet = first[2..6].iter().chain(&first[8..]);
    assert!(
        walls_and_outlet.into_iter().all(|f| f.abs() < 1e-9),
        "{first:?}"
    );
    assert!(first[6] < 0.0, "{first:?}");

    // A second run into the same directory needs --force.
    let again = run(
        &small_channel(),
        &dir,
        &["--backend", "cpu", "--end-time", "0.01"],
    );
    assert!(!again.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_aborts_cleanly_when_the_solution_diverges_cpu() {
    aborts_on_divergence("cpu");
}

#[test]
fn run_aborts_cleanly_when_the_solution_diverges_wgpu() {
    aborts_on_divergence("wgpu");
}

#[test]
fn run_aborts_cleanly_when_the_solution_diverges_cuda() {
    aborts_on_divergence("cuda");
}

fn aborts_on_divergence(backend: &str) {
    let Some(mut args) = backend_args(backend) else {
        return;
    };
    let dir = temp_dir(&format!("nan-{backend}"));
    let mut scene = small_channel();
    // τ ≈ 0.506 with BGK at Ma ≈ 0.26: accepted with warnings, but blows up.
    scene.fluids[0].kinematic_viscosity = 6.7e-7;
    args.extend([
        "--collision",
        "bgk",
        "--lattice-velocity",
        "0.15",
        "--end-time",
        "20",
        "--output-interval",
        "0.3",
    ]);
    let out = run(&scene, &dir, &args);
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let m = meta(&dir);
    assert_eq!(m["run"]["status"], "diverged");
    assert!(m["run"]["message"]
        .as_str()
        .unwrap()
        .contains("stability.tau"));
    let frames = m["run"]["frames"].as_u64().unwrap() as usize;
    assert!(frames >= 1);
    assert_eq!(count(&dir.join("out/frames")), frames);
    // Every written frame is finite.
    for e in std::fs::read_dir(dir.join("out/frames")).unwrap() {
        let f = cfd_io::frame::Frame::decode(&std::fs::read(e.unwrap().path()).unwrap()).unwrap();
        assert!(f
            .fields
            .iter()
            .all(|f| f.values.iter().all(|v| v.is_finite())));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Whether this build has the CUDA backend and the machine can run it.
fn cuda_available() -> Result<(), String> {
    #[cfg(feature = "cuda")]
    return cfd_cuda::CudaDevice::new(&cfd_cuda::CudaOptions::default())
        .map(|_| ())
        .map_err(|e| e.to_string());
    #[cfg(not(feature = "cuda"))]
    Err(cfd_cuda::CudaError::NotBuilt.to_string())
}

/// `--backend auto` takes CUDA, else a hardware wgpu adapter, else the CPU. Each engine is made
/// unavailable in turn: CUDA through a device ordinal that does not exist, wgpu through an
/// adapter filter that matches nothing.
#[test]
fn auto_falls_back_from_cuda_to_wgpu_to_cpu() {
    let hardware_wgpu = cfd_gpu::adapters().iter().any(|a| !cfd_gpu::is_software(a));
    let without_cuda = if hardware_wgpu { "wgpu" } else { "cpu" };
    // (name, environment, expected backend)
    type Case = (
        &'static str,
        &'static [(&'static str, &'static str)],
        &'static str,
    );
    let cases: [Case; 3] = [
        (
            "all",
            &[],
            if cuda_available().is_ok() {
                "cuda"
            } else {
                without_cuda
            },
        ),
        ("no-cuda", &[("CFD_CUDA_DEVICE", "99")], without_cuda),
        (
            "neither",
            &[
                ("CFD_CUDA_DEVICE", "99"),
                ("CFD_WGPU_ADAPTER", "no adapter has this name"),
            ],
            "cpu",
        ),
    ];
    for (name, env, expected) in cases {
        let dir = temp_dir(&format!("auto-{name}"));
        let args = ["--backend", "auto", "--end-time", "0.02"];
        let out = run_env(&small_channel(), &dir, &args, env);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{name}: {stderr}");
        assert_eq!(
            meta(&dir)["solver"]["backend"],
            expected,
            "{name}: {stderr}"
        );
        if name != "all" {
            assert!(
                stderr.contains("note: CUDA unavailable"),
                "{name}: {stderr}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
