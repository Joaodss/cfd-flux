# Benchmarks

Solver throughput in **MLUPS** (million fluid-lattice updates per second; one update = one cell,
one time step, both lattices for thermal runs). Measured with `cfd-cli bench`:

```bash
cargo run --release -p cfd-cli -- bench --backend cpu             # all logical cores
cargo run --release -p cfd-cli -- bench --backend cpu --threads 8 # Phase 1 acceptance: ≥ 50 MLUPS on 8 cores
cargo run --release -p cfd-cli -- bench --backend wgpu --sizes 256,512,1024,2048,4000
cargo run --release -p cfd-cli -- bench --backend wgpu --workgroup 256   # other workgroup sizes
cargo run --release -p cfd-cli -- bench --backend cuda --sizes 256,512,1024,2048,4000
cargo run --release -p cfd-cli -- bench --json out.json          # machine-readable results
```

- **Scenes:** lid-driven cavity at Re = 100 (isothermal, D2Q9) and differentially heated cavity at
  Ra = 1e5, Pr = 0.71 (thermal, D2Q9 + D2Q5), N² fluid cells plus walls, built by
  `cfd_verify::scenes`, so the boundary links are included.
- **Method:** a 0.3 s warm-up, then a fixed number of steps chosen so each run lasts ≥ 2 s,
  repeated 3 times; the median is reported, with min and max. Results are reproducible within a few
  percent on an idle machine (the 8-thread 256² case is the noisiest). GPU runs are timed until the
  queued steps have finished (`Solver::finish`).
- Add a new section per machine/backend; keep old ones for comparison.

## CUDA vs wgpu — NVIDIA GeForce RTX 4060 (8 GB), 2026-10-12

Windows 11, driver 610.62 (CUDA 13.3), NVRTC 13.4 (`sm_89` cubin), wgpu 30 on Vulkan, block/workgroup
128, TRT collision, Phase 2b code. Two rounds, the four configurations interleaved within each round
(the card also drives a 144 Hz desktop, so runs vary by a few percent); each cell is
`round 1 / round 2`, MLUPS (median of 3).

```bash
cargo run --release -p cfd-cli -- bench --backend cuda --sizes 256,512,1024,2048,4000
cargo run --release -p cfd-cli -- bench --backend cuda --fmad            # fused multiply-adds
cargo run --release -p cfd-cli -- bench --backend cuda --graph-steps 0   # one launch per step
```

| Model | Grid (fluid) | CUDA (default) | CUDA `--fmad` | CUDA `--graph-steps 0` | wgpu |
|-------|--------------|---------------:|--------------:|-----------------------:|-----:|
| isothermal (D2Q9) | 256² | 9269 / 9673 | 9138 / 9466 | 6034 / 6038 | 9314 / 8946 |
| isothermal (D2Q9) | 512² | 7454 / 7132 | 6721 / 7232 | 7092 / 7128 | 6999 / 7130 |
| isothermal (D2Q9) | 1024² | 2608 / 2735 | 2603 / 2734 | 2727 / 2754 | 2835 / 2831 |
| isothermal (D2Q9) | 2048² | 2593 / 2702 | 2583 / 2744 | — | 2729 / 2842 |
| isothermal (D2Q9) | 4000² | 2632 / 2737 | 2594 / 2773 | — | 2846 / 2856 |
| thermal (D2Q9 + D2Q5) | 256² | 5759 / 6183 | 5985 / 6240 | 4642 / 4691 | 5623 / 5653 |
| thermal (D2Q9 + D2Q5) | 512² | 1762 / 1866 | 1865 / 1888 | 1787 / 1849 | 1935 / 1916 |
| thermal (D2Q9 + D2Q5) | 1024² | 1693 / 1728 | 1749 / 1783 | 1730 / 1786 | 1855 / 1863 |
| thermal (D2Q9 + D2Q5) | 2048² | 1698 / 1739 | 1799 / 1798 | — | 1852 / 1866 |
| thermal (D2Q9 + D2Q5) | 4000² | 1715 / 1818 | 1811 / 1772 | — | 1881 / 1834 |

**Reading the numbers**

- **Phase 2 criterion (≥ 1,000 MLUPS at 1024²): met** on CUDA too — 2,600–2,750 isothermal and
  1,690–1,790 thermal; NFR-01's ≥ 2,000 is met for the isothermal model.
- **Both engines are bandwidth-bound from 1024² on**, and the kernels are equally fast: Nsight Systems
  shows 83% of the CUDA steps at 1024² taking 360–380 µs (≈ 2,850 MLUPS, ≈ 245 GB/s, 90% of the card's
  272 GB/s), which is wgpu's rate. The 3–8% that CUDA loses in wall time comes from the display: the
  same GPU drives a 144 Hz desktop, and every ~6.9 ms (one refresh) the compositor time-slices the
  running CUDA kernel, stretching ~8% of the steps by 200–600 µs and leaving short idle gaps (the GPU
  is busy 97.4% of the time). wgpu's Vulkan queue is apparently interleaved with the compositor more
  cheaply. A headless GPU (or one not driving a display) should remove the difference.
- **Launch overhead:** a CUDA kernel launch costs ≈ 8 µs of CPU time on Windows (WDDM), more than a
  whole step at 256² (≈ 7 µs on the GPU), so with one launch per step the GPU waits for the CPU (6,000
  vs 9,300 MLUPS). Replaying 50 captured steps per CUDA graph launch (the default) brings the CPU cost
  to ≈ 0.6 µs per step and 256² back to wgpu's rate; wgpu gets the same effect by recording up to 1000
  dispatches in one command buffer.
- **512² (in the 24 MB L2 cache):** CUDA is on par with or faster than wgpu.
- **FMA (`--fmad`)** makes no consistent difference (the kernel waits on memory, not on arithmetic),
  so the default keeps `--fmad=false`, which makes CUDA bit-identical to the CPU backend (ADR-021).
- Block size (64–512 threads) changes nothing beyond the noise, as with wgpu.

## wgpu backend — NVIDIA GeForce RTX 4060 (8 GB), 2026-10-10

Windows 11, driver 610.62, Vulkan, wgpu 30, workgroup 128, TRT collision, commit `eec034b` + CLI
wiring (Phase 2a). The largest grid is 4000² because scenes are limited to 4096 cells per side
(walls included).

| Model | Grid (fluid) | MLUPS (median) | min | max |
|-------|--------------|---------------:|----:|----:|
| isothermal (D2Q9) | 256² | 9340 | 9306 | 9388 |
| isothermal (D2Q9) | 512² | 7443 | 7413 | 7474 |
| isothermal (D2Q9) | 1024² | 2834 | 2834 | 2857 |
| isothermal (D2Q9) | 2048² | 2855 | 2852 | 2856 |
| isothermal (D2Q9) | 4000² | 2856 | 2844 | 2868 |
| thermal (D2Q9 + D2Q5) | 256² | 5638 | 5634 | 5694 |
| thermal (D2Q9 + D2Q5) | 512² | 1912 | 1895 | 1923 |
| thermal (D2Q9 + D2Q5) | 1024² | 1812 | 1788 | 1853 |
| thermal (D2Q9 + D2Q5) | 2048² | 1854 | 1831 | 1855 |
| thermal (D2Q9 + D2Q5) | 4000² | 1890 | 1882 | 1890 |

**Reading the numbers**

- **Phase 2 criterion (≥ 1,000 MLUPS at 1024²): met** — 2,834 isothermal, 1,812 thermal; NFR-01's
  ≥ 2,000 on the RTX 4060 is met for the isothermal model. 22× the CPU backend (16 threads) at 1024².
- From 1024² on, the kernel is limited by memory bandwidth: an isothermal update moves ≈ 86 B
  (36 B of populations in, 36 B out, 12 B of ρ/u, 2 B of flags), so 2,850 MLUPS is ≈ 245 GB/s, 90%
  of the card's 272 GB/s. A thermal update moves ≈ 130 B (≈ 240 GB/s at 1,850 MLUPS).
- At 256² (both models) and 512² (isothermal) the working set stays in the 24 MB L2 cache, so
  the rate is far above the DRAM limit; the thermal working set at 512² (≈ 34 MB) no longer fits.
- The workgroup size hardly matters (32 → 512 invocations: within ±2% at 1024² and 2048²), and DX12
  gives the same rates as Vulkan (2,830 / 2,856 isothermal at 1024² / 2048²).
- Halving the traffic is the next lever: the AA pattern or Esoteric Pull (one population buffer) and
  f16 population storage (plan, Phase 2a optional item).

## CPU backend — AMD Ryzen 7 3700X (8 cores / 16 threads), 2026-10-10

Windows 11, DDR4 (dual channel), Rust 1.99.0 (MSVC), release profile, TRT collision, commit `c8348a8`.

| Model | Grid (fluid) | 16 threads | 8 threads | 1 thread |
|-------|--------------|-----------:|----------:|---------:|
| isothermal (D2Q9) | 256² | 165.8 | 136.6 | 30.9 |
| isothermal (D2Q9) | 512² | 180.0 | 117.0 | — |
| isothermal (D2Q9) | 1024² | 126.0 | 93.7 | 12.2 |
| isothermal (D2Q9) | 2048² | 131.0 | 100.3 | — |
| thermal (D2Q9 + D2Q5) | 256² | 126.6 | 91.7 | 19.2 |
| thermal (D2Q9 + D2Q5) | 512² | 101.3 | 71.6 | — |
| thermal (D2Q9 + D2Q5) | 1024² | 84.8 | 65.5 | 8.8 |
| thermal (D2Q9 + D2Q5) | 2048² | 87.3 | 68.6 | — |

**Reading the numbers**

- **Phase 1 criterion (≥ 50 MLUPS on 8 cores): met** for every size and both models (≥ 65 MLUPS).
- Up to 512² the two population buffers (2 × 9 × 4 B per cell ≈ 19 MB at 512²) fit in the 32 MB L3
  cache and the kernel is compute-bound. From 1024² on it streams from DRAM: an isothermal update moves
  ≈ 2 × 36 B of populations plus ≈ 18 B of flags and macroscopic fields, so 130 MLUPS is ≈ 12 GB/s
  (about a third of the theoretical DDR4 bandwidth; the rest is lost to the pull pattern and to
  write-allocate traffic).
- The thermal model costs ~1.5× (14 populations instead of 9).
- A single core does ~31 MLUPS in cache and ~12 MLUPS from DRAM; 16 threads (SMT) add ~30% over 8.
