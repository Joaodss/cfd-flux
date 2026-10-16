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
cargo run --release -p cfd-cli -- bench --backends cuda,wgpu     # interleaved, one column per backend
```

- **Scenes:** lid-driven cavity at Re = 100 (isothermal, D2Q9) and differentially heated cavity at
  Ra = 1e5, Pr = 0.71 (thermal, D2Q9 + D2Q5), N² fluid cells plus walls, built by
  `cfd_verify::scenes`, so the boundary links are included.
- **Method:** a 0.3 s warm-up, then a fixed number of steps chosen so each run lasts ≥ 2 s,
  repeated 3 times; the median is reported, with min and max. Results are reproducible within a few
  percent on an idle machine (the 8-thread 256² case is the noisiest). GPU runs are timed until the
  queued steps have finished (`Solver::finish`).
- `--backends a,b` builds one solver per backend for each case, warms each up, then alternates the
  timed repetitions (a, b, a, b, …), so whatever else the machine does affects every column alike.
- Add a new section per machine/backend; keep old ones for comparison.

## Run loop and server — NVIDIA GeForce RTX 4060 (8 GB), 2026-10-13

Phase 3 code (`cfd_job::run`: batches of at most ≈ 50 ms, each followed by `finish`, frames on an
output thread). Release builds, normal desktop (not quiet).

- **Cost of the command loop:** a 1024² lid-driven cavity through `cfd-cli run --backend cuda --no-frames`
  runs at **2,878 MLUPS**, against 2,885 for `bench` at the same size (≈ 0.2%).
- **Streaming the cylinder** (442 × 84, `outputInterval` = 0.01 s = 60 steps, frames stored as f32 and
  streamed as f16; `cfd-cli submit scenes/cylinder-re100.json --cancel-after 3` against `cfd-server`):

  | Engine | First frame after submitting | Frames/s received | Skipped by backpressure | Cancel took effect after |
  |--------|------------------------------|-------------------|-------------------------|--------------------------|
  | CUDA | 0.18 s | 37.5 | 2 of 113 | 46 ms |
  | wgpu | 0.38 s | 38.6 | 2 of 106 | 40 ms |
  | CPU (16 threads) | 0.04 s | 29.4 | 2 of 91 | 30 ms |

  At this output interval the **output thread** sets the pace, not the solver: encoding a frame with the
  pure-Rust `ruzstd` takes 12.6 ms (f32, stored) + 7.2 ms (f16, streamed) and compresses these fields by only
  3–4%; the solver waits rather than drop a stored frame (open question Q13 in
  [07](07-decisions-and-questions.md)). Without frames, 6,000 steps of the same CUDA loop take 0.38 s of
  wall time (5,180 MLUPS of solver time) with diagnostics, forces and probes sampled every 60 steps, and
  0.70 s (2,080 MLUPS) with the default sampling every 6 steps (output interval / 10): batches then end every
  6 steps, and each sample (a reduction, the forces and a copy of the fields for the probes) costs ≈ 0.26 ms
  on this grid.

## On-device output sampling — NVIDIA GeForce RTX 4060 (8 GB) / Ryzen 7 3700X, 2026-10-16

One output frame of a lid-driven cavity as a server job takes it: the stored frame (velocity, pressure,
vorticity; f32, full resolution) and one stream format (the same fields, f16, box-downsampled to ≤ 1024 cells
per side), produced by `Solver::request_samples` (ADR-028) or by the CPU path it replaces (`request_fields`,
`FieldSet::from_lattice`, the box filter and the f16 conversion on the output thread). Medians of 7 frames,
release build, nothing queued on the GPU; "copy" is the wait for the device's work and the transfer (in a job it
overlaps with the next batch of steps), "conversion" is the output thread's work before encoding.

```bash
cargo run --release -p cfd-cli --example sampling -- cuda,wgpu,cpu 1024,2048
```

| Engine | Grid | Path | Copy (wait) | Conversion | Encoding (2 frames) | Total |
|---|---|---|---:|---:|---:|---:|
| cuda | 1024×1025 | `request_samples` (device) | 7.29 ms | 0.00 ms | 5.90 ms | 13.19 ms |
| cuda | 1024×1025 | CPU conversion | 4.49 ms | 32.52 ms | 6.24 ms | 43.25 ms |
| wgpu | 1024×1025 | `request_samples` (device) | 9.60 ms | 0.00 ms | 6.05 ms | 15.65 ms |
| wgpu | 1024×1025 | CPU conversion | 7.35 ms | 32.70 ms | 6.37 ms | 46.42 ms |
| cpu | 1024×1025 | `request_samples` (CPU default) | 1.76 ms | 34.84 ms | 5.86 ms | 42.46 ms |
| cuda | 2048×2049 | `request_samples` (device) | 26.80 ms | 0.00 ms | 19.27 ms | 46.07 ms |
| cuda | 2048×2049 | CPU conversion | 17.38 ms | 121.43 ms | 20.39 ms | 159.20 ms |
| wgpu | 2048×2049 | `request_samples` (device) | 33.82 ms | 0.00 ms | 19.33 ms | 53.15 ms |
| wgpu | 2048×2049 | CPU conversion | 27.62 ms | 121.75 ms | 20.57 ms | 169.94 ms |
| cpu | 2048×2049 | `request_samples` (CPU default) | 7.19 ms | 126.36 ms | 20.49 ms | 154.04 ms |

- The output thread's work per frame drops 3.3× (1024²) to 3.5× (2048²); what is left is encoding (byte shuffle +
  lz4 of 25 MB at 2048²). The device copy is longer than the lattice copy: the samples are 1.5× the bytes (16 B
  per cell f32 + 8 B f16 stream vs 16 B of ρ, u, θ) plus the kernel.
- **Server stream of the cylinder** (`cfd-cli submit scenes/cylinder-re100.json --cancel-after 3`, two runs): CUDA
  194–222 frames/s (codec only: 162–187), wgpu 101–110, CPU 51–52. At 442 × 84 the output thread no longer sets
  the pace; the run loop does (its diagnostics, forces and probe samples every 6 steps end the batches).

## Frame codec (LFFR v2) — AMD Ryzen 7 3700X, 2026-10-16

Release build, `cfd-cli`'s `frame_codec` example on stored frames (velocity, pressure, vorticity = 4 values per
cell) of the cylinder at t = 0.5 s (442 × 84) and of a Re = 100 lid-driven cavity at 1024 × 1025 (CUDA). Median
times; encoding runs the per-field lz4 blocks on rayon's pool (one thread in brackets).

```bash
cargo run --release -p cfd-cli --example frame_codec -- cavity-scene cavity-1024.json
cargo run --release -p cfd-cli -- run cavity-1024.json --out cavity --backend cuda
cargo run --release -p cfd-cli --example frame_codec -- bench cyl/frames/000050.bin cavity/frames/000004.bin
```

| Frame | v1: `ruzstd` fastest | v2: shuffle + `lz4_flex` | Decode v1 → v2 |
|-------|---------------------:|-------------------------:|---------------:|
| Cylinder f32 (594 kB raw) | 96.7%, 13.3 ms | 76.4%, 0.47 ms (0.59) | 1.16 → 0.39 ms |
| Cylinder f16 (297 kB raw) | 96.3%, 7.3 ms | 71.3%, 0.32 ms (0.40) | 0.42 → 0.45 ms |
| 1024² f32 (16.8 MB raw) | 91.3%, 328 ms | 63.7%, 13.6 ms (21.9) | 34 → 17 ms |
| 1024² f16 (8.4 MB raw) | 80.7%, 158 ms | 49.9%, 10.2 ms (14.9) | 30 → 17 ms |

- **The shuffle:** a blocked loop (64 values per block, fixed inner trip counts, no bounds checks) takes 4.8 ms for
  the 1024² f32 fields into a reused buffer (6.8 ms with a fresh allocation, whose page faults cost ≈ 2 ms) against
  13.8 ms for the plain indexed loop. Baseline x86-64 has no byte-shuffle instruction (`pshufb` is SSSE3), so LLVM
  cannot do much better: with `-C target-cpu=x86-64-v3` the same loop takes 3.0 ms, and a SWAR variant (4 values →
  one `u32` per byte plane) 3.3 ms at baseline. At ≈ 10 GB/s of memory traffic the rest is bandwidth.
- Packing the values (f32 → bytes, or f32 → f16 with `half`'s F16C path) adds 0.04–0.3 ms on the cylinder and
  6–7 ms at 1024² (allocation included).

**Server stream** (`cfd-cli submit scenes/cylinder-re100.json --cancel-after 3`, frames every 60 steps, stored as
f32 and streamed as f16; two runs):

| Engine | Frames/s v1 (2026-10-13) | Frames/s v2 | First frame | Cancel took effect after |
|--------|---:|---:|---:|---:|
| CUDA | 37.5 | 162–187 | 0.05 s | 19–26 ms |
| wgpu | 38.6 | 99–111 | 0.03 s | 17–21 ms |
| CPU (16 threads) | 29.4 | 50 | 0.04 s | 112–164 ms |

Encoding no longer sets the pace: the output thread's remaining work per frame is the conversion to physical
fields (`FieldSet::from_lattice`), the f16 conversion and the box filter on the CPU — the on-device sampling
kernels' job (Phase 3).

## Frame encoding (open question Q13) — AMD Ryzen 7 3700X, 2026-10-13

One thread, release build, a throwaway benchmark on two real frames (velocity, pressure, vorticity = 4 values
per cell): the cylinder at t = 0.5 s (442 × 84) and the Re = 100 lid-driven cavity at 1024 × 1025. Size is the
compressed payload as a share of the raw bytes; "shuffle" is the HDF5/Blosc byte shuffle (all first bytes of
every value, then all second bytes, …), a plain scalar loop here (0.33 ms / 13 ms for the f32 frames).

| Codec | Cylinder f32 | Cylinder f16 | 1024² f32 | 1024² f16 | Decode 1024² f32 |
|-------|--------------|--------------|-----------|-----------|------------------|
| `ruzstd` fastest (now) | 96.7%, 12.1 ms | 96.3%, 7.0 ms | 88.3%, 305 ms | 81.7%, 142 ms | 30 ms |
| shuffle + `ruzstd` fastest | 79.4%, 11.2 ms | 74.2%, 6.4 ms | 63.8%, 247 ms | 53.6%, 115 ms | — |
| C `zstd` level 1 | 90.1%, 1.2 ms | 90.6%, 0.7 ms | 79.9%, 29 ms | 72.0%, 18 ms | 21 ms |
| shuffle + C `zstd` level 1 | 73.6%, 1.4 ms | 66.3%, 1.0 ms | 59.7%, 34 ms | 46.7%, 17 ms | 21 ms |
| shuffle + C `zstd` level 3 | 73.0%, 2.4 ms | 64.2%, 1.3 ms | 57.6%, 57 ms | 43.2%, 25 ms | — |
| `lz4_flex` | 97.0%, 0.27 ms | 97.0%, 0.15 ms | 92.0%, 9.8 ms | 83.1%, 8.3 ms | 4 ms |
| shuffle + `lz4_flex` | 76.4%, 0.74 ms | 71.2%, 0.55 ms | 64.0%, 25 ms | 51.1%, 13 ms | 17 ms |
| none | 100%, 0 | 100%, 0 | 100%, 0 | 100%, 0 | 0 |

Building the raw bytes adds 0.1–0.3 ms (cylinder) and 4 ms (f32) / 8 ms (f16 conversion) at 1024².

- The byte shuffle is what makes these fields compress (the high bytes of neighbouring floats are similar; the
  low mantissa bytes are noise): 25–35% smaller with every codec.
- `ruzstd` is ≈ 10× slower than C `zstd` for the same ratio; shuffle + pure-Rust `lz4_flex` is within 3–5
  points of shuffle + C `zstd` level 1 and as fast or faster (the shuffle loop dominates; it can be vectorised).

## CUDA vs wgpu, interleaved — NVIDIA GeForce RTX 4060 (8 GB), 2026-10-13

Windows 11, driver 617.42 (CUDA 13.4), NVRTC 13.4 (`sm_89` cubin), wgpu 30 on Vulkan, block/workgroup
128, TRT collision, Phase 2c code (commit `ef53348`). **Quiet desktop:** display at 50 Hz, nothing
animating. One run, the two engines interleaved, 5 repetitions of ≥ 2 s each per case; median
(min–max), MLUPS. 3 min 12 s in total.

```bash
cargo run --release -p cfd-cli -- bench --backends cuda,wgpu --sizes 256,512,1024,2048,4000 --repeat 5
```

| Model | Grid (fluid) | cuda MLUPS (min–max) | wgpu MLUPS (min–max) | wgpu / cuda |
|-------|--------------|---:|---:|---:|
| isothermal (D2Q9) | 256² | 9941 (9912–9984) | 9854 (9825–9876) | 0.991 |
| isothermal (D2Q9) | 512² | 8507 (8405–8508) | 7318 (7289–7330) | 0.860 |
| isothermal (D2Q9) | 1024² | 2873 (2873–2873) | 2877 (2876–2877) | 1.001 |
| isothermal (D2Q9) | 2048² | 2875 (2874–2875) | 2867 (2866–2868) | 0.997 |
| isothermal (D2Q9) | 4000² | 2881 (2881–2882) | 2879 (2862–2879) | 0.999 |
| thermal (D2Q9 + D2Q5) | 256² | 6471 (6339–6486) | 5489 (5344–5518) | 0.848 |
| thermal (D2Q9 + D2Q5) | 512² | 1979 (1976–1979) | 1933 (1928–1934) | 0.977 |
| thermal (D2Q9 + D2Q5) | 1024² | 1880 (1880–1880) | 1884 (1883–1884) | 1.002 |
| thermal (D2Q9 + D2Q5) | 2048² | 1863 (1863–1863) | 1869 (1869–1869) | 1.003 |
| thermal (D2Q9 + D2Q5) | 4000² | 1897 (1897–1898) | 1899 (1899–1899) | 1.001 |

**Reading the numbers**

- **From 1024² on, the two engines are the same speed** (within ±0.3%): both kernels are
  DRAM-bound at ≈ 245 GB/s, 90% of the card's 272 GB/s (the CUDA kernel moves exactly the ideal
  84 B per isothermal update, see below). On a quiet desktop the repetitions of a case agree to
  within 0.6% at these sizes; the few-percent differences between the engines in the earlier,
  non-interleaved tables were desktop noise.
- **CUDA is ahead only while the working set sits in the 24 MB L2 cache:** +16% at 512² isothermal
  and +18% at 256² thermal, where the kernel is no longer bandwidth-bound and launch/dispatch costs
  and code generation matter (CUDA replays 50 steps per graph launch; wgpu records up to 1000
  dispatches per command buffer). At 512² thermal (≈ 34 MB, out of cache) the gap closes again.
- The **cost of portability** for this solver is therefore nil at production sizes: wgpu/WGSL runs
  on any GPU at the CUDA rate, while CUDA keeps the bit-identity with the CPU reference (ADR-021) and
  a lead on small, cache-resident grids.

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
- **The CUDA kernel runs at the card's memory limit.** Nsight Compute on one 1024² isothermal step
  (driver 617.42): 355.8 µs (≈ 2,950 MLUPS), **DRAM throughput 93.8% of peak** (247 GB/s at the P2 memory
  clock), 39.9 MB read + 48.1 MB written = **84 B per cell, exactly the ideal traffic** (36 B of
  populations in and out, 12 B of ρ/u, 2 B of flags), SM throughput 18%, 40 registers per thread, 100%
  theoretical / 85% achieved occupancy. Only moving fewer bytes (AA pattern, f16 storage) can make it
  faster; FMA cannot (the arithmetic units are mostly idle).
- **Wall-clock rates depend on what the desktop is drawing.** The same GPU drives a 144 Hz display, and
  once per refresh (every ~6.9 ms) the Windows compositor (`dwm`) preempts the running kernel. Nsight
  Systems shows the undisturbed steps at 360–400 µs and one step per refresh stretched by what the
  compositor needs: 200–600 µs in the run above, ~1.6 ms (≈ 23% of the GPU) in a later run while `dwm`
  used 41% of the 3D engine (an animated window on screen), when both backends dropped to ~2,150–2,250
  MLUPS at 1024². The kernels of the two engines are equally fast; the few-percent differences between
  them in the table are within this desktop noise. For clean numbers, benchmark with the screen still
  (nothing animating or playing video), or on a GPU that does not drive a display.

**Quiet desktop (display at 50 Hz, nothing animating), driver 617.42, two interleaved rounds:**

| Model | Grid (fluid) | CUDA | wgpu |
|-------|--------------|-----:|-----:|
| isothermal (D2Q9) | 256² | 10161 / 10109 | 9825 / 9125 |
| isothermal (D2Q9) | 512² | 8232 / 8228 | 7562 / 6941 |
| isothermal (D2Q9) | 1024² | 2873 / 2873 | 2969 / 2737 |
| isothermal (D2Q9) | 2048² | 2852 / 2852 | 2960 / 2730 |
| isothermal (D2Q9) | 4000² | 2882 / 2882 | 2964 / 2733 |
| thermal (D2Q9 + D2Q5) | 256² | 6505 / 6419 | 5387 / 4977 |
| thermal (D2Q9 + D2Q5) | 512² | 1979 / 1945 | 1994 / 1849 |
| thermal (D2Q9 + D2Q5) | 1024² | 1880 / 1879 | 1943 / 1942 |
| thermal (D2Q9 + D2Q5) | 2048² | 1865 / 1865 | 1923 / 1923 |
| thermal (D2Q9 + D2Q5) | 4000² | 1897 / 1748 | 1960 / 1960 |

- The compositor no longer shows up: Nsight Systems finds 5 preempted steps per second (vs ~145 at
  144 Hz) costing 0.1% of the time, the GPU busy 99.5%, and `dwm` below 0.5% of the 3D engine.
- CUDA now reaches **2,870 MLUPS at 1024²**, 97% of the kernel's own rate (2,950 MLUPS in Nsight
  Compute), and repeats to 4 digits between rounds. Both engines run at ≈ 245–255 GB/s, 90–94% of the
  card's bandwidth; which one is a few percent ahead varies between runs (wgpu here 2,740–2,970).
  CUDA is ahead on the small, cache-resident grids (256², 512²).
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
