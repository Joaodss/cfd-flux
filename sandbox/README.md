# sandbox — learning prototypes

Throwaway code for learning Rust and GPU programming before writing the real solver (Phase 0 of the
[plan](../docs/05-implementation-plan.md)). Each prototype is an independent Cargo project,
**outside** the main workspace (`cargo new sandbox/<name>`), so it is not part of CI.

| # | Prototype | Goal | Done |
|---|-----------|------|------|
| 1 | `lbm-minimal` | D2Q9 BGK LBM in a single `main.rs`: 128² lid-driven cavity, writes a velocity PNG (`image` crate). First sequential, then `rayon` (`par_chunks_mut`). Measure MLUPS. | ⬜ |
| 2 | `wgpu-hello` | Add two vectors in a WGSL compute shader and read the result back to the CPU. | ⬜ |
| 3 | `cuda-hello` | The same with `cudarc` and a `.cu` kernel compiled by NVRTC (needs the CUDA Toolkit). | ⬜ |
| 4 | `lbm-gpu` | Port the kernel from prototype 1 to wgpu **or** CUDA and compare MLUPS with the CPU. | ⬜ |

Tips for C# developers: see "Notes for C# developers" in the plan.
