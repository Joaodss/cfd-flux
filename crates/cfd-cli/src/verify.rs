//! `cfd-cli verify`: runs the validation suite of `cfd-verify` on a backend and writes the report.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::Result;
use cfd_core::Domain;
use cfd_verify::harness::Ctx;
use cfd_verify::{cases, report, Level};
use clap::Args;

use crate::backend::SolverArgs;

#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Low resolutions and a subset of cases, fast enough for CI (the default).
    #[arg(long, conflicts_with = "full")]
    pub quick: bool,
    /// Every case at the resolutions needed for the published tolerances.
    #[arg(long)]
    pub full: bool,
    /// Only these cases (ids or id prefixes, e.g. `cavity`); runs them even if not in `--quick`.
    #[arg(long, value_delimiter = ',')]
    pub cases: Option<Vec<String>>,
    /// List the cases and exit.
    #[arg(long)]
    pub list: bool,
    /// Report directory.
    #[arg(long, default_value = "validation/report")]
    pub report: PathBuf,
    /// Do not write a report.
    #[arg(long)]
    pub no_report: bool,
    #[command(flatten)]
    pub solver: SolverArgs,
}

pub fn verify(args: VerifyArgs) -> Result<ExitCode> {
    let level = if args.full { Level::Full } else { Level::Quick };
    let all = cases::all();
    if args.list {
        for c in &all {
            let tag = if c.quick { "quick+full" } else { "full" };
            println!("{:<22} {:<11} {}", c.id, tag, c.title);
        }
        return Ok(ExitCode::SUCCESS);
    }
    let selected: Vec<&cases::Case> = match &args.cases {
        Some(names) => {
            let picked: Vec<_> = all
                .iter()
                .filter(|c| names.iter().any(|n| c.id.starts_with(n.as_str())))
                .collect();
            if picked.is_empty() {
                anyhow::bail!("no case matches {names:?}; see `cfd-cli verify --list`");
            }
            picked
        }
        None => all.iter().filter(|c| c.in_level(level)).collect(),
    };

    let choice = args.solver.resolve(None)?;
    // The factory every case uses to build its solvers: a closure capturing the backend choice.
    let factory = |d: Domain| choice.make(d);
    let log = |msg: &str| eprintln!("{msg}");
    let ctx = Ctx::new(&factory, level, &log);
    eprintln!(
        "verify ({level:?}): {} cases on the {} backend, {}, {} threads",
        selected.len(),
        choice.backend,
        choice.collision,
        choice.threads
    );

    let t = Instant::now();
    let results: Vec<_> = selected.iter().map(|c| cases::run(c, &ctx)).collect();
    let seconds = t.elapsed().as_secs_f64();

    println!("\n| Case | Result | Checks | Time |");
    println!("|------|--------|--------|------|");
    for r in &results {
        let checked = r.metrics.iter().filter(|m| m.passed().is_some()).count();
        let passed = r
            .metrics
            .iter()
            .filter(|m| m.passed() == Some(true))
            .count();
        println!(
            "| {} | {} | {passed}/{checked} | {:.1} s |",
            r.id,
            if r.passed() { "pass" } else { "**FAIL**" },
            r.seconds
        );
    }
    let failed = results.iter().filter(|r| !r.passed()).count();
    println!(
        "\n{} of {} cases passed in {seconds:.0} s",
        results.len() - failed,
        results.len()
    );

    if !args.no_report {
        let info = report::RunInfo {
            level,
            backend: choice.backend.into(),
            collision: choice.collision.into(),
            threads: choice.threads,
            cpu: crate::bench::cpu_name(),
            generator: format!("cfd-cli {}", env!("CARGO_PKG_VERSION")),
            commit: git_commit(),
            seconds,
        };
        report::write(&args.report, &info, &results)?;
        eprintln!(
            "report written to {}",
            args.report.join("README.md").display()
        );
    }
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Short hash of the checked-out commit (plus `-dirty` with local changes), if git is available.
fn git_commit() -> Option<String> {
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git").args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let hash = git(&["rev-parse", "--short", "HEAD"])?;
    let dirty =
        git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    Some(if dirty { format!("{hash}-dirty") } else { hash })
}
