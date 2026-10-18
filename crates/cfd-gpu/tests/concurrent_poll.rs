//! Readbacks on a device that another thread polls at the same time.
//!
//! `device.poll` collects the map callbacks of every finished submission and runs them after it
//! returns, on the thread that polled. A `PollType::Wait` on one thread can therefore return
//! before the callback it waited for has run on another thread; `Mapping::wait` must keep waiting
//! for it. The parity tests, which share one device across test threads, poll concurrently like
//! this.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cfd_core::solver::Solver;
use cfd_core::{Domain, DomainOptions};
use cfd_gpu::{GpuContext, GpuOptions, WgpuLbm};
use cfd_lbm::LbmConfig;

#[test]
fn readbacks_survive_concurrent_polls() {
    let opts = GpuOptions {
        allow_software: true,
        ..Default::default()
    };
    let ctx = match GpuContext::new(&opts) {
        Ok(c) => c,
        Err(e) if std::env::var_os("CFD_REQUIRE_GPU").is_some() => {
            panic!("CFD_REQUIRE_GPU is set but there is no wgpu adapter: {e}")
        }
        Err(e) => {
            eprintln!("skipping: {e}");
            return;
        }
    };
    let scene = cfd_verify::scenes::periodic_box(16, 1e-3);
    let opts = DomainOptions {
        lattice_velocity: 0.05,
        characteristic_velocity: None,
    };
    let domain = Domain::from_scene(&scene, &opts).expect("domain");

    let stop = Arc::new(AtomicBool::new(false));
    let pollers: Vec<_> = (0..2)
        .map(|_| {
            let (ctx, stop) = (ctx.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = ctx.device.poll(wgpu::PollType::Poll);
                }
            })
        })
        .collect();

    let mut solver = WgpuLbm::new(ctx.clone(), domain, LbmConfig::default()).expect("solver");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut rounds = 0;
    let result = loop {
        // One batch per step call, then a blocking status readback and a field readback.
        let r = solver.step(1).and_then(|()| solver.finish());
        if r.is_err() || Instant::now() > deadline {
            break r;
        }
        assert!(solver
            .lattice_fields()
            .density
            .iter()
            .all(|v| v.is_finite()));
        rounds += 1;
    };
    stop.store(true, Ordering::Relaxed);
    for p in pollers {
        p.join().unwrap();
    }
    result.unwrap_or_else(|e| panic!("after {rounds} rounds: {e}"));
    eprintln!("{rounds} readback rounds under concurrent polls");
}
