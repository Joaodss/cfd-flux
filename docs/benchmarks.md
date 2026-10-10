# Benchmarks

Solver throughput in **MLUPS** (million fluid-lattice updates per second; one update = one cell,
one time step, both lattices for thermal runs). Measured with `cfd-cli bench`:

```bash
cargo run --release -p cfd-cli -- bench                 # all logical cores
cargo run --release -p cfd-cli -- bench --threads 8     # Phase 1 acceptance: ≥ 50 MLUPS on 8 cores
cargo run --release -p cfd-cli -- bench --json out.json # machine-readable results
```

- **Scenes:** lid-driven cavity at Re = 100 (isothermal, D2Q9) and differentially heated cavity at
  Ra = 1e5, Pr = 0.71 (thermal, D2Q9 + D2Q5), N² fluid cells plus walls, built by
  `cfd_verify::scenes`, so the boundary links are included.
- **Method:** a 0.3 s warm-up, then a fixed number of steps chosen so each run lasts ≥ 2 s,
  repeated 3 times; the median is reported, with min and max. Results are reproducible within a few
  percent on an idle machine (the 8-thread 256² case is the noisiest).
- Add a new section per machine/backend; keep old ones for comparison.

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
