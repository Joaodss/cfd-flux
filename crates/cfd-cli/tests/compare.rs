//! End-to-end tests of `cfd-cli compare` on a small channel.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::{available, small_channel, temp_dir};

fn compare(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let scene = dir.join("scene-in.json");
    std::fs::write(&scene, small_channel().to_json_pretty()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_cfd-cli"))
        .arg("compare")
        .arg(&scene)
        .arg("--out")
        .arg(dir.join("out"))
        .args(["--threads", "2", "--steps", "200"])
        .args(args)
        .envs(env.iter().copied())
        .output()
        .expect("cfd-cli runs")
}

fn report(dir: &Path) -> (String, serde_json::Value) {
    let out = dir.join("out");
    let md = std::fs::read_to_string(out.join("README.md")).unwrap();
    let json = std::fs::read_to_string(out.join("compare.json")).unwrap();
    (md, serde_json::from_str(&json).unwrap())
}

fn diff_maps(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("out"))
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with("-diff.png")
        })
        .count()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn cpu_against_itself_is_bit_identical() {
    let dir = temp_dir("compare-cpu");
    let out = compare(&dir, &["--backends", "cpu,cpu"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let (md, json) = report(&dir);
    assert!(
        md.contains("| cpu-2 |") && md.contains("✅ bit-identical"),
        "{md}"
    );
    let case = &json["cases"][0];
    assert_eq!(case["plan"]["steps"], 200);
    assert_eq!(case["runs"][0]["label"], "cpu");
    assert_eq!(case["runs"][1]["steps"], 200);
    let c = &case["comparisons"][0];
    assert_eq!(c["backend"], "cpu-2");
    assert_eq!(c["identical"], true);
    assert_eq!(c["passed"], true);
    assert_eq!(c["fields"][1]["field"], "ux");
    assert_eq!(c["fields"][1]["metric"], 0.0);
    // No difference maps for identical fields; the reference speed and the series are drawn.
    assert_eq!(diff_maps(&dir), 0);
    for f in [
        "scene.json",
        "reference-speed.png",
        "kinetic-energy.svg",
        "forces.svg",
    ] {
        assert!(dir.join("out").join(f).exists(), "{f}");
    }

    // A second report into the same directory needs --force (exit code 2: not compared).
    let again = compare(&dir, &["--backends", "cpu,cpu"], &[]);
    assert_eq!(again.status.code(), Some(2), "{}", stderr(&again));
    let forced = compare(&dir, &["--backends", "cpu,cpu", "--force"], &[]);
    assert!(forced.status.success(), "{}", stderr(&forced));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cuda_is_bit_identical_to_the_cpu() {
    if !available("cuda") {
        return;
    }
    let dir = temp_dir("compare-cuda");
    // A tolerance of 0 requires bit-identical fields.
    let out = compare(&dir, &["--backends", "cpu,cuda", "--tolerance", "0"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));
    let (_, json) = report(&dir);
    assert_eq!(json["cases"][0]["comparisons"][0]["identical"], true);
    assert_eq!(diff_maps(&dir), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wgpu_is_within_the_tolerance() {
    if !available("wgpu") {
        return;
    }
    let dir = temp_dir("compare-wgpu");
    let out = compare(
        &dir,
        &[
            "--backends",
            "cpu,wgpu",
            "--allow-software",
            "--maps",
            "all",
        ],
        &[],
    );
    assert!(out.status.success(), "{}", stderr(&out));
    let (_, json) = report(&dir);
    let c = &json["cases"][0]["comparisons"][0];
    assert_eq!(c["passed"], true);
    for f in c["fields"].as_array().unwrap() {
        assert!(f["metric"].as_f64().unwrap() < 1e-5, "{f}");
    }
    // `--maps all`: ρ and |Δu| even where identical.
    assert_eq!(diff_maps(&dir), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unavailable_or_too_few_backends_are_errors() {
    let dir = temp_dir("compare-errors");
    // A CUDA device ordinal that does not exist (or a build without CUDA).
    let out = compare(
        &dir,
        &["--backends", "cpu,cuda"],
        &[("CFD_CUDA_DEVICE", "99")],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("cannot open the cuda backend"),
        "{}",
        stderr(&out)
    );

    let out = compare(&dir, &["--backends", "cpu"], &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("at least two backends"),
        "{}",
        stderr(&out)
    );

    // `auto` is not a backend here; --validation takes no scene options.
    let out = compare(&dir, &["--backends", "cpu,auto"], &[]);
    assert!(!out.status.success());
    let out = Command::new(env!("CARGO_BIN_EXE_cfd-cli"))
        .args(["compare", "--validation", "--steps", "10"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("cannot be used with"),
        "{}",
        stderr(&out)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
