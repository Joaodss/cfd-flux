//! CUDA graphs replay captured steps: the results must be exactly those of single launches, also
//! for odd step counts and when a parameter changes between calls (the graph is recaptured).
#![cfg(feature = "cuda")]

use cfd_core::{Domain, DomainOptions, Solver};
use cfd_cuda::{CudaDevice, CudaLbm, CudaLbmOptions, CudaOptions};

#[test]
fn graphs_match_single_launches() {
    let dev = match CudaDevice::new(&CudaOptions::default()) {
        Ok(d) => d,
        Err(e) if std::env::var_os("CFD_REQUIRE_CUDA").is_none() => {
            return eprintln!("skipping: {e}")
        }
        Err(e) => panic!("CFD_REQUIRE_CUDA is set: {e}"),
    };
    let opts = DomainOptions {
        lattice_velocity: 0.05,
        characteristic_velocity: None,
    };
    let d = Domain::from_scene(&cfd_verify::scenes::plane_channel(24, 0.1, 0.0), &opts).unwrap();
    let solver = |graph_steps| {
        let o = CudaLbmOptions {
            graph_steps,
            ..Default::default()
        };
        CudaLbm::with_options(dev.clone(), d.clone(), Default::default(), o).unwrap()
    };
    let (mut single, mut graph) = (solver(0), solver(10));
    for s in [&mut single, &mut graph] {
        // Odd chunks: graphs only start on even steps; the force change forces a recapture.
        s.set_body_force([1e-5, 0.0]);
        s.step(7).unwrap();
        s.step(123).unwrap();
        s.set_body_force([2e-5, 0.0]);
        s.step(101).unwrap();
        s.finish().unwrap();
    }
    assert_eq!(graph.steps_done(), 231);
    let f = graph.lattice_fields();
    assert_eq!(f, single.lattice_fields());
    assert!(
        f.ux.iter().any(|&u| u > 0.0),
        "the body force drives a flow"
    );
}
