# 06 — Advanced optimizations and AI

Everything here is **post-MVP** unless stated otherwise. Roughly ordered by benefit/cost.

## 1. Performance optimizations

### 1.1 Memory and bandwidth (LBM is bandwidth-bound)
| Technique | Expected gain | Notes |
|-----------|---------------|-------|
| Fused stream-collide kernel + SoA | Baseline | Already in the MVP |
| **AA pattern / Esoteric Pull** | −50% memory, up to +30% speed | A single `f_i` buffer |
| **f16/"FP16C" storage** (compute in f32) | −50% memory, ~1.5–2× speed | FluidX3D technique; validate accuracy |
| Compact flags + precomputed solid neighbours | Less divergence | Branch-free kernels |
| **Block-sparse grid** (only blocks containing fluid) | Large for solid-heavy domains | 16×16/32×32 tiles + active-block table |

### 1.2 Algorithms
- Geometric/algebraic **multigrid** for Poisson (projection solver, FLIP).
- **Adaptive time stepping** (CFL) for explicit methods that allow it; in LBM, rescale `u_lb`.
- **Local refinement (AMR)** with quadtrees/octrees: multi-level LBM (grid refinement with `τ` rescaling) — fine cells near walls and wakes, coarse elsewhere.
- **Dynamic meshes:** periodic refinement/coarsening driven by vorticity, temperature gradients or phase interfaces.
- **Turbulence:** LES (Smagorinsky, WALE) and wall functions for high Re at low resolution.

### 1.3 Hardware
- **Multi-GPU:** domain decomposition into strips/blocks with halo exchange (1 cell in LBM); later multi-node (MPI or gRPC/QUIC).
- **CUDA-specific optimizations** (the base CUDA backend exists from Phase 2): shared memory for tiling, warp shuffles, Tensor Cores for AI inference, CUDA Graphs to reduce launch overhead, multi-GPU with NCCL/peer-to-peer.
- **CPU:** explicit SIMD (AVX2/AVX-512/NEON), cache tiling, NUMA awareness.
- **In-browser simulation** (WebGPU): the same WGSL running locally for fast low-resolution previews, no server needed.

## 2. Alternatives to the uniform grid

| Representation | When to use | Integration with the pixel editor |
|----------------|-------------|-----------------------------------|
| Uniform grid (MVP) | Always, as the baseline | Direct |
| **Quadtree/octree AMR** | Variable resolution | Pixels define the geometry at the finest level |
| **Immersed boundary / interpolated bounce-back** | Curved walls without "staircases" | Extract a sub-pixel contour from the pixels (marching squares + smoothing) |
| **Cut-cell** (finite volumes) | Accurate walls on a Cartesian grid | Same as above |
| **Unstructured mesh** (triangles/polygons; FV or DG) | Complex geometry, anisotropic refinement | Contour → mesh generator (constrained Delaunay, e.g. `spade`) |
| **Meshless** (SPH, MPM) | Violent liquids, granular media, deformable solids | Pixels → initial particles |
| **High-order methods** (DG, spectral elements) | Accuracy per degree of freedom | Requires an unstructured mesh |

The editor stays pixel-based; **conversion to another representation** happens on the solver side (`Scene → Domain` gets several implementations). Optionally, vector tools in the editor (Bézier curves) for smooth geometry.

## 3. AI

### 3.1 Data pipeline
1. Procedural scene generator (random obstacles, inlets, temperatures) in `tools/datagen`.
2. Bulk runs with the GPU solver (batch mode, no streaming) → dataset of (scene, fields over time) pairs.
3. Storage in Zarr/HDF5 with metadata (Re, Ra, method, resolution).
4. Offline training in Python/PyTorch; export to ONNX.
5. Inference in the Rust worker via `ort` (ONNX Runtime, CUDA/DirectML/CPU) or `burn`.

### 3.2 Uses, from safest to most ambitious
| Use | Description | Risk to accuracy |
|-----|-------------|------------------|
| **Warm start** | AI predicts an approximate steady state; the classic solver converges from there | None (the solver corrects it) |
| **Instant preview** | A surrogate shows a prediction in < 100 ms while the user draws | Labelled as an "AI preview" |
| **Learned preconditioner / Poisson** | A network accelerates the most expensive step of projection methods | Low (final iterations are classic) |
| **Super-resolution** | Simulate on a coarse grid and reconstruct the detail | Medium |
| **Learned turbulence closure** | Sub-grid model trained on fine simulations | Medium/high |
| **Full surrogate** (FNO, U-Net, MeshGraphNets, operator transformers) | Advances many steps at once | High — always with a confidence indicator and a "verify with solver" option |

### 3.3 Candidate architectures
- **U-Net** conditioned on the geometry mask and parameters (Re, Ra) — simple and strong on grids.
- **Fourier Neural Operator (FNO)** — resolution-independent.
- **MeshGraphNets / GNNs** — for unstructured meshes and AMR.
- **Diffusion models** for super-resolution and generating turbulent fields.

### 3.4 Evaluation
Physical metrics, not just pixel metrics: mass conservation, divergence, C_D/St/Nu error on the validation cases, stability over long rollouts. A model is only enabled by default if it meets defined thresholds.

## 4. Suggested post-MVP priorities

1. f16 storage + AA pattern (big gain, low risk)
2. LES + robust TRT/MRT (unlocks realistic Re)
3. Interactive mode (Phase 5b — very visual, great for the portfolio)
4. Block-sparse grid
5. AI warm start and preview
6. Quadtree AMR
7. Multi-GPU
8. Unstructured meshes / 3D
