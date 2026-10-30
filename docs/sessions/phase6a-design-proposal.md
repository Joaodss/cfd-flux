# Phase 6a — free-surface liquids (the last part of the MVP): design proposal (2026-10-30)

**Status: proposed, awaiting the author's review** ([#42](https://github.com/Joaodss/cfd-flux/issues/42); tracking
issue [#41](https://github.com/Joaodss/cfd-flux/issues/41)). No solver code is written yet. Once the questions of §17
are answered, the accepted parts move into [04 §3/§5](../04-numerical-methods.md), [03](../03-data-model.md) and a new
ADR, and the 6a issues (#43–#48) and the plan's 6a tasks follow §16.

Goal of Phase 6a ([plan](../05-implementation-plan.md#6a--free-surface-mvp-adr-008), ADR-008): liquids with a free
surface (water in "empty" gas), on all three engines, validated. **Proposed acceptance (§15):** open the *Dam break*
example in the editor, run it on wgpu and on CUDA, watch the column collapse live, and read a front position x(t)
within 5 % of Martin & Moyce (1952) and a liquid-mass drift below 0.1 %.

What exists already (survey of 2026-10-30, issue #42): the scene format (`Physics::free_surface`, `CellType::Empty = 5`,
`OutputField::FillFraction`), the `dam_break()` example (202 × 101 cells, dx = 2.5 mm, a 40 × 80-cell column), the
editor's *Empty* palette entry and *Free surface* checkbox, and the viewer's `fillFraction` quantity. Validation rejects
these scenes (`support.freeSurface`, `domain.rs:737`), and the fill fraction is skipped wherever it would be produced.

## 1. Overview

The method is the standard free-surface LBM of Körner et al. (2005), as refined by Thürey (2007) and implemented on GPUs
by FluidX3D (Lehmann). Only the liquid is simulated. The gas is "empty": it has a constant pressure and no inertia.
Each non-solid cell is in one of three dynamic states:

```
   gas (G)            interface (I)                    liquid (L)
   not updated        0 < φ < 1, own mass m            φ = 1, m = ρ
   φ = 0, m = 0       populations from gas links are   ordinary LBM cell
                      reconstructed; m is advected by
                      the population exchange
```

One time step is four passes over the grid, each a per-cell *gather* that reads only what earlier passes wrote:

```
 state S0, f_src, m, massex, φ
   │
   ├─ 1. stream_collide  pull + collide (as today) on L and I cells; gas links reconstructed;
   │                     mass exchange Δm; mark I cells that fill (IF) or empty (IG)          → f_dst, m, ρ, u, S1
   ├─ 2. surface_fill    gas next to an IF cell → GI (new interface); IG next to IF → back to I  → S2
   ├─ 3. surface_init    GI cells: equilibrium from the neighbours' mean ρ, u; L next to IG → I  → f_dst(GI), S3
   └─ 4. surface_finish  IF → L, IG → G, GI → I; excess mass m − ρ (or m < 0) set aside and its
                         receiver count stored; φ = m/ρ                                       → m, massex, φ, S4 (= next S0)
```

This is FluidX3D's structure (`stream_collide`, `surface_1`, `surface_2`, `surface_3`), with one change that our pull
scheme allows: FluidX3D's `surface_0` (mass exchange and gas reconstruction before streaming, needed by its in-place
Esoteric-Pull storage) folds into the pull of pass 1, because our source buffer still holds every post-collision
population of the previous step (§2.3).

### Rust, GPU and parallel idioms used below (with .NET parallels)

- **Ping-pong buffers:** a pass reads one array and writes another, and the two swap roles for the next pass, like
  double-buffering a `WriteableBitmap`, or a `Parallel.For` that writes into a new array instead of in place. This is
  how `f` works today. §3 applies it to the cell states too. In-place updates would be a **data race**: one thread
  writes a cell's state while a neighbour's thread reads it.
- **Why the CPU needs it too:** the CPU kernel splits the output into disjoint `&mut [f32]` row blocks (rayon's
  `par_chunks_mut`) and reads the source through a shared `&[f32]`. The borrow checker accepts this because no cell is
  both read and written. In-place neighbour reads are not expressible in safe Rust. They would need `AtomicU32`
  (≈ `Interlocked`) or `unsafe`, and the workspace denies `unsafe_code` outside `cfd-cuda/src/raw.rs`.
- **WGSL has no byte stores:** storage buffers hold 32-bit words. Two invocations writing different bytes of one word
  race, which is why the `u16` flags are read-only on the GPU (two cells per word, ADR-019). New per-cell state that
  kernels *write* must be one `u32` per cell.
- **Bindings:** a WGSL kernel sees buffers through numbered *bindings*. WebGPU guarantees only 8 storage buffers per
  shader stage, and `stream_collide` uses exactly 8 today (ADR-019). §3.3 shows how the free surface fits in them.

## 2. The method (finding 1)

### 2.1 References and what was checked

| Source | Used for | Checked for this proposal |
|--------|----------|---------------------------|
| Körner, Thies, Hofmann, Thürey, Rüde, *J. Stat. Phys.* 121 (2005) 179–196 | The model: mass per cell, fill fraction, interface cells, gas-side reconstruction, cell conversion | **No.** Paywalled; not reachable from this session. The formulas below are the standard ones and must be checked against the paper in #43 before coding |
| Thürey, PhD thesis, Erlangen (2007) | Refinements: conversion offsets, normal-weighted excess, "lonely" cells, LES for stability | **No.** The PDF host refused the connection. Same caveat |
| FluidX3D, `src/kernel.cpp` and `src/defines.hpp` (master, read 2026-10-30) | A GPU implementation: pass structure, conversion rules, excess-mass split | **Yes**, the source code. Details in §2.2–2.6 are FluidX3D's unless marked otherwise |
| Martin & Moyce, *Phil. Trans. R. Soc. A* 244 (1952) 312–324 | Dam-break reference data | **No** (403 from the publisher). The data must be digitised from the paper in #45 (§10.3) |

### 2.2 Mass exchange

Each interface cell carries a mass `m` (lattice units, liquid density ≈ 1) and a fill fraction `φ = m/ρ`. Along each
link `i` it exchanges, with the population `f_i^in` that streams in from neighbour `x − c_i` and the population
`f*_ī(x)` that left along the opposite link:

| Neighbour across link i | Δm_i |
|-------------------------|------|
| liquid | `f_i^in − f*_ī(x)` |
| interface | `½ (φ(x) + φ(x − c_i)) · (f_i^in − f*_ī(x))` |
| gas, wall, outside | 0 |

Then `m(t+1) = m(t) + Σ_i Δm_i + (excess mass gathered from neighbours, §2.5)`. Liquid cells run the same sum, so that
`m − ρ` becomes their excess. FluidX3D does not test the neighbour type for liquid cells, which relies on the invariant
that a liquid cell never touches gas.

**Exact antisymmetry (a design rule for parity):** the weight must use the *stored* φ of both cells (written by pass 4
of the previous step). Then the exchange seen from the two sides is `w·(a − b)` and `w·(b − a)`, exact negatives in IEEE
arithmetic, and mass is conserved link by link up to the rounding of the final sums. FluidX3D recomputes the cell's own
φ from its current mass, which makes the two sides differ slightly. We should not copy that.

### 2.3 Gas-side reconstruction

The gas sends no populations. For an interface cell, the population that would come from a gas neighbour is rebuilt
so that the gas imposes its pressure (ρ_G = 1, i.e. gauge 0) and the interface's own velocity:

`f_i^in = f_i^eq(ρ_G, u) + f_ī^eq(ρ_G, u) − f*_ī(x)`

- **Which links.** FluidX3D rebuilds only the links that come from gas cells. Körner/Thürey also rebuild the links with
  `n·c_ī > 0`, where n is the interface normal, so that a cell has enough information on the gas side. **Proposed:**
  gas links only, as FluidX3D (no normal needed, §2.6). If #43 shows asymmetries or noise at the surface, the
  normal-based rule is the fallback (Q2).
- **Which velocity.** FluidX3D takes u from the cell's own outgoing (post-collision) populations plus the Guo half-force
  shift, not from the stored u field. **Proposed:** the same. In our pull scheme all nine `f*(x)` are in the source
  buffer at the cell's own index, so this costs nine extra loads, only on interface cells. It does not depend on the
  macroscopic stores that `skip_macro` (ADR-029) skips.
- **Where.** Inside the pull of pass 1, as one more link-wise rule next to bounce-back and anti-bounce-back
  (ADR-014): the link's source is a gas cell (from the state plane) instead of a wall (from the static flags). Gas cells
  are never updated, and their populations are never read.
- Surface tension would enter here as the Laplace pressure, an offset of the gas density by `Δρ = σκ/c_s² = 3σκ` in
  lattice units (FluidX3D: `1 − 6σκ`, with its own curvature convention). It is 6b, not 6a.

### 2.4 Cell conversion

Interface cells that are full become liquid, and empty ones become gas (FluidX3D, verified):

- **IF** (interface → liquid): `m > ρ`, or no gas neighbour ("lonely" interface inside the liquid).
- **IG** (interface → gas): otherwise, `m < 0`, or no liquid neighbour (an isolated drop cell). IF wins when both hold.
- Comparisons are strict, with no offset. Thürey uses a small offset κ (fill when `m > (1 + κ)ρ`, empty when
  `m < −κρ`, κ ≈ 10⁻³ as usually quoted, not re-checked) to stop cells from flipping back and forth. **Proposed:** start without κ (FluidX3D), and keep κ
  as a solver constant that #43 can raise if flipping shows up in the hydrostatic test (§10.2).
- Neighbours keep the layer closed: a gas cell next to an IF cell becomes interface (**GI**), initialised with the
  equilibrium of the *mean* ρ and u of its liquid/interface neighbours. A liquid cell next to an IG cell becomes
  interface. An IG cell next to an IF cell is cancelled (stays interface).

### 2.5 Excess mass

A converted cell rarely has exactly `m = ρ` (filled) or `m = 0` (emptied). The difference (and, for interface cells,
any `m > ρ` or `m < 0`) is set aside as excess and handed to the neighbours in the next step:

| Option | Receivers | Weights | Notes |
|--------|-----------|---------|-------|
| **A — FluidX3D (proposed)** | liquid and interface neighbours (L, I, IF, GI) | equal split | Verified code. Simplest. Excess received by liquid cells is passed on again until an interface cell absorbs it. The total is conserved, it just travels |
| B — Thürey | interface neighbours only | `n·c_i` (filled cells: towards +n; emptied: towards −n) | Needs the normal in pass 4. Said to reduce surface artefacts (not re-checked) |
| C — equal split over interface neighbours only | I, GI | equal | Between A and B; no normal |

**Accounting change (proposed):** FluidX3D stores each cell's *share* (`massex / count`). We store the undivided excess
and the receiver count (4 bits of the state word, §3.1), and the receiver divides as it gathers (eight IEEE divisions,
deterministic on the CPU and CUDA). Then the total liquid mass is exactly `Σ (m + massex)` at any step. That is what
`diagnostics.csv` reports and what the 0.1 % drift criterion is measured on (§9.2). A cell with no receivers keeps its
excess (FluidX3D, verified).

### 2.6 Interface normal and flags

- **Normal:** with gas-link reconstruction (§2.3) and the equal split (§2.5 A), the MVP **does not need a normal**. It
  comes with surface tension in 6b (FluidX3D uses the Parker–Youngs stencil for the normal and a PLIC least-squares fit
  for the curvature). Options B in §2.5 and the Körner reconstruction rule would bring it forward: central differences
  of φ, 3 × 3 cells.
- **Flags:** the static `u16` flags stay geometry only. In a free-surface domain the *Empty* cells are static `FLUID`
  (their link masks mark walls, as for any fluid cell), and the dynamic G/I/L state lives in its own plane (§3.1).
  Nothing in the static flags changes during a run.

## 3. Data layout (finding 5)

### 3.1 New per-cell arrays (identical on all three backends)

| Array | Type | Written by | Read by | Content |
|-------|------|-----------|---------|---------|
| `state[2]` | `u32` ×2 (ping-pong) | passes 1–4 | passes 1–4 | bits 0–1 G/I/L; bits 2–3 pending IF/IG/GI; bits 4–7 receiver count of the excess (0–8); bits 8–31 free (6b: contact angle, …) |
| `mass` | `f32` | passes 1, 4 (own cell) | passes 1, 4 (own cell) | m |
| `massex` | `f32` | pass 4 | pass 1 (neighbours) | undivided excess (§2.5) |
| `phi` | `f32` | pass 4 | pass 1 (neighbours), sampling | φ: 1 liquid, m/ρ interface, 0 gas and solid |

One buffer `surf` holds the five planes in that order (`n` words each, SoA like `f`): `state_a, state_b, mass, massex,
phi`. The CPU keeps the same planes as `Vec<u32>`/`Vec<f32>` and uploads them unchanged (ADR-019). Plane offsets and the
state bits come from `cfd_lbm::abi`, like the existing flag bits (never typed twice).

ρ and u: in free-surface mode pass 1 stores ρ on every liquid and interface cell every step (pass 4 needs it for
`m − ρ` and φ), and u on interface cells (pass 3 averages it). `skip_macro` (ADR-029) keeps skipping u on liquid cells.
That is 4 B of the 12 it saved.

**Why two state planes instead of one byte (FluidX3D's `uchar flags`):** byte writes race in WGSL (§1), and in-place
neighbour reads are a data race on all three backends and cannot be written in safe Rust. FluidX3D gets away with
in-place flags because every predicate a neighbour evaluates during a pass is the same for the old and the new value
(for example "is it gas?" is false for both I and IF). That works on GPUs in practice, but it is formally undefined in
the WGSL and CUDA memory models. Ping-pong costs 4 B/cell and makes every pass a pure function of the previous one, so
the CPU and CUDA can be bit-identical (§8).

### 3.2 Memory (NFR-04: 4096² in 8 GB)

| | Bytes stored per cell | 4096² (16.8 M cells) |
|---|---|---|
| Today, isothermal: two f32 populations (72) + flags (2) + slot (4) + ρ/u/θ planes (16) | 94 | 1.58 GB |
| Free surface: + state ×2 (8) + mass (4) + massex (4) + φ (4) | **114** | **1.91 GB** |
| FluidX3D's extension, for comparison: mass, massex, φ (+ its 1-byte flags in the base) | +12 | |
| After Phase 8a (Esoteric Pull + f16 populations): 40 + 20 | 60 | 1.0 GB |

Free surface excludes the thermal lattice (§11), so the 140 B/cell thermal case never adds to it. NFR-04 holds with a
wide margin. Bytes **moved** per cell and step grow from 72–84 to an estimated ≈ 150 B: f (72), state read/written in
four passes (32), mass (8), ρ (4), neighbours' massex and φ (8, mostly cached), pass 4 (≈ 20). So a free-surface step
costs roughly twice a single-phase step on the bandwidth-bound GPUs. That is irrelevant for the dam break (20 k cells,
launch-bound) and acceptable elsewhere.

### 3.3 wgpu bindings

`stream_collide` binds 8 storage buffers (flags, slot, bc, f_src, f_dst, g_src, g_dst, mac). Free surface excludes
the thermal lattice, so **the free-surface pipeline binds `surf` where the thermal pipeline binds `g_src`** and leaves
`g_dst` as the 16-byte dummy: still 8. That needs a second shader module (`lbm_fs.wgsl`, sharing `common.wgsl` and the
generated prelude), because one WGSL module cannot declare two different variables at the same binding for the same
entry point. Passes 2–4 bind `surf`, `mac`, `flags` and the populations only, well under 8. The context already requests
the adapter's own limits (Vulkan on the RTX 4060 allows far more), but staying within WebGPU's 8 keeps a future
in-browser engine possible (ADR-010) and keeps lavapipe CI and DX12 safe.

### 3.4 Phase 8a (Esoteric Pull, f16)

FluidX3D combines Esoteric Pull, FP16C storage and the free surface, so the combination is proven. Under Esoteric Pull,
populations are overwritten in place, so the gas reconstruction and mass exchange move back into a separate pass before
streaming (FluidX3D's `surface_0`). The arithmetic stays the same, so the "Esoteric Pull is bit-identical to ping-pong"
test of 8a still applies. The free surface makes 8a about 20 % more work (four more kernels per backend to port). The
plan's risk table already lists this.

## 4. GPU passes (race freedom)

Every pass runs one invocation per cell (CPU: per row block), skips solid cells (static flags) and reads only buffers
that it does not write, or its own cell's word of a buffer it writes:

| Pass | Cells | Reads | Writes |
|------|-------|-------|--------|
| 1 `stream_collide` | L, I (G cells only copy their state word) | `f_src` (neighbours and own), `state_a` (own + 8 neighbours), `mass` (own), `massex`, `phi` (8 neighbours) | `f_dst` (own), `mass` (own), ρ/u (own), `state_b` (own: + IF/IG mark) |
| 2 `surface_fill` | all non-solid | `state_b` (own + 8) | `state_a`: G with an IF neighbour → GI; IG with an IF neighbour → I |
| 3 `surface_init` | all non-solid | `state_a` (own + 8), ρ/u (8 neighbours of GI cells) | `state_b`; `f_dst`, ρ, u of GI cells (equilibrium of the mean ρ, u of L/I/IF neighbours, fixed direction order); L with an IG neighbour → I |
| 4 `surface_finish` | all non-solid | `state_b` (own + 8), `mass`, ρ (own) | `state_a` (final G/I/L + receiver count), `mass`, `massex`, `phi` (own) |

`state_a` holds the step's input and output, so no swap is needed between steps. Pass 3 writes `f_dst` only for GI
cells, which pass 1 did not touch (they were gas). The same holds for ρ/u. CUDA: four kernels, captured in the existing
50-step graph (200 launches), so the WDDM launch cost stays amortised (ADR-021). wgpu: four dispatches per step in the
same command buffers (ADR-020).

Invariants checked by a debug pass in the CPU tests (not in release kernels): no liquid cell has a gas neighbour; every
interface cell has `0 ≤ m ≤ ρ` after pass 4; gas cells have `m = massex_received = 0`.

## 5. Units and time step (finding 3)

Today the characteristic velocity (`domain.rs` ≈ l. 349) takes only prescribed velocities and Boussinesq buoyancy.
For the dam break it is 0, so `dt = dx²/(6ν)` = **1.04 s** (τ = 1), and the first step jumps a second ahead.

**Proposed:** with a free surface and gravity, add a **free-fall velocity** `U_g = √(2 g H_l)` to the candidates, where
`H_l` is the extent of the initial liquid along ĝ (bounding box of the *Fluid* cells projected on g). It is the speed of
a particle falling the column's height, the scale of every gravity-driven liquid flow here.

| Dam break (H_l = 0.2 m, dx = 2.5 mm) | u_lb(U_g) = 0.05 (default, **proposed**) | u_lb(U_g) = 0.1 |
|---|---|---|
| U_g = √(2gH) | 1.98 m/s | 1.98 m/s |
| dt | 6.31e-5 s | 1.26e-4 s |
| steps for the 1 s run | 15,845 | 7,922 |
| g_lb = g dt²/dx | 1.56e-5 | 6.25e-5 |
| Ritter front speed 2√(gH) = 2.80 m/s → Ma | 0.12 | 0.24 |
| Hydrostatic compressibility Δρ/ρ = 3 g_lb H_lb | 0.375 % | 1.5 % |

Two general results follow from `U_g ↦ u_lb`:

- `g_lb = u_lb² / (2 H_lb)`, so the **hydrostatic density variation is `Δρ/ρ = 1.5 u_lb²`, at every resolution.** At the
  default 0.05 it is 0.375 %. Liquid *volume* (Σφ) therefore changes by up to ≈ 0.4 % as the column collapses while the
  mass is conserved. The 0.1 % criterion must be on mass, not volume (§9.2).
- **Mach margin:** the front accelerates towards the Ritter speed 2√(gH) = √2·U_g, and the run-up jet at the far wall is
  faster still. At 0.05 the front sits at Ma 0.12, which leaves room below `MACH_WARN = 0.17`. At the 0.1 in the
  issue's estimate the front alone would reach Ma 0.24. The existing max-Mach diagnostic warns at run time if the splash
  exceeds the limit.

The override (`DomainOptions::characteristic_velocity`, `--velocity` in the CLI) still applies.

## 6. Gravity and the output pressure (finding 4)

- **In the solver:** with a free surface, gravity is a **body force on the liquid**: `LatticePhysics::body_force =
  g_lb` (the field exists, used today only by validation cases), applied through the existing Guo forcing as
  `F = ρ g_lb`. Boussinesq stays off (thermal is out of scope, §11). The hydrostatic pressure now *forms* in the
  liquid as a density stratification of 0.375 % (§5).
- **ADR-016 is amended, not replaced:** single-phase runs keep the hydrostatic split. Free-surface runs set
  `Hydrostatic::gravity = 0`, so the sampled pressure is directly `p = c_s² (ρ − 1) ρ₀ (dx/dt)²`: the **gauge pressure
  relative to the gas**. At the bottom of the dam-break column that is ρ₀ g H = 1,958 Pa (check: Δρ = 3.75e-3 gives
  1,958 Pa with dx/dt = 39.6 m/s). In gas cells p = 0 is physically right (atmosphere), not a mask value.
- **Initial state (proposed):** initialise the liquid **hydrostatically**, `ρ = 1 + 3 g_lb·d`, with d the depth below
  the first non-liquid cell above it along −ĝ (column by column, which handles overhangs). A uniform ρ = 1 start would
  release a pressure wave of amplitude 0.375 % at t = 0. That wave is small, but it is visible in probes and in the
  first frames, and the hydrostatic start is what Martin & Moyce's gate held. Done in `cfd_lbm::shared::initial_fields`,
  so all three backends start from the same bytes.

## 7. Viscosity and stability (finding 2)

Water's ν = 1.004e-6 m²/s at dx = 2.5 mm and dt = 6.31e-5 s gives ν_lb = 1.01e-5 and **τ = 0.50003**. `TAU_MIN =
0.505` rejects the scene today (an error, `stability.tau`). The column Reynolds number U a/ν (U = 2 m/s, a = 0.1 m) is
≈ 2 × 10⁵, and the grid Reynolds number U dx/ν ≈ 5,000 (`GRID_RE_WARN = 30`). The flow is far from resolved. The
front, though, is a gravity–inertia balance and nearly inviscid: viscosity enters through a bottom boundary layer of
√(ν t) ≈ 0.45 mm at t = 0.2 s, less than a cell. Other presets are not affected: engine oil gives τ = 0.527, glycerol
0.536. Ethanol (0.50005) has water's problem.

| Option | What | τ at the dam break | Cost on the three backends | Physics |
|--------|------|-------------------|----------------------------|---------|
| **A — Smagorinsky LES (proposed)** | `τ_eff = ½(τ₀ + √(τ₀² + 18√2 C_s² ‖Π^neq‖/ρ))` (Hou et al. 1996), ‖Π^neq‖ the Frobenius norm of the non-equilibrium momentum flux. With TRT, τ⁺ = τ_eff and ω⁻ from Λ per cell | 0.50003 where the flow is smooth; ≈ 0.503–0.506 in the shear at the front (C_s 0.1–0.17, \|S\|_lb ≈ 0.07) | ≈ 30 flops, one `sqrt`, one division per cell; no memory. CPU and CUDA stay bit-identical (IEEE `sqrt`/division with `--fmad=false`); WGSL `sqrt` is not correctly rounded, so wgpu stays within its tolerance | Keeps water's ν where it matters; the model the plan already lists (04 §3.2, Phase 8). Also useful for high-Re gases |
| B — effective viscosity | Run with a larger ν: τ = 0.51 needs ν_eff = 3.3e-4 m²/s (329 × water) | 0.51 | None (a scene choice) | Re ≈ 600; the boundary layer grows to √(ν t) ≈ 8 mm (3 cells) at 0.2 s and will slow the tip; to be measured against the 5 % |
| C — finer grid | τ ≥ 0.505 with u_lb = 0.1 needs dx ≈ 30 µm | 0.505 | 16,600 × 8,300 cells = 1.4 × 10⁸ cells, 13 GB at 94 B/cell: **does not fit** | — |
| D — TRT's free parameter | Λ = 1/4 (best linear stability) instead of 3/16 | 0.50003 | None | At τ⁺ = 0.50003 the antisymmetric rate is ω⁻ ≈ 1.6e-4 whatever Λ; **not enough alone**, combinable with A |
| E — a more robust collision (regularised, MRT, cumulant) | Replace TRT for free-surface runs | 0.50003 | A new collision on three backends, and new parity work | Most robust; too large for the MVP |

**Proposed: A**, as a scene option (it changes results, so it travels with the scene, like the reference scales of
Phase 5): `physics.turbulence: { "model": "smagorinsky", "constant": 0.1 }`, optional, additive (schema version stays
1). The dam-break example sets it. `check_stability` keeps rejecting τ < 0.505 without a model (with the hint "enable
the Smagorinsky model") and only warns with one. B stays available by editing the fluid's viscosity. **Risk:** whether
A alone is stable at τ₀ = 0.50003 is not known before #43 runs it. The first CPU milestone runs the dam break with A
and with B, and the measured front of both decides (Q4). If A is not stable alone, A + a ν floor (a minimum τ₀ such as
0.5005) is the fallback.

## 8. Determinism and parity (finding 6)

- **CPU ↔ CUDA can stay bit-identical.** Every pass is a per-cell gather over a fixed neighbour order, from buffers that
  the pass does not write. There are no atomics, no list compaction and no order-dependent sums: the excess gather, the
  GI average and the mass exchange all sum in link order 1…8. Divisions and the LES `sqrt` are IEEE on both, and CUDA
  compiles with `--fmad=false` (ADR-021). The condition: the CPU runs **the same four passes**, not Thürey's sequential
  lists of filled and emptied cells. A sequential CPU version would make different conversion decisions where several
  cells convert at once, and parity would be lost for good.
- **Discrete decisions amplify rounding.** A conversion is a threshold (`m > ρ`), so a one-ulp difference flips a cell,
  and the interface then differs by a cell from that step on. On wgpu (driver FMA contraction, ≤ 3e-6 today) **the
  fields will not stay within 1e-5** beyond the first hundreds of steps of a dam break, much as the cylinder wake drifts
  today (ADR-023), only sooner.
- **Proposed tolerances:** CUDA: `cfd-cli compare --tolerance 0` (bit-identical) on the free-surface parity cases and
  the dam break, as on every validation geometry today. wgpu: the 1e-5 field metric on short parity cases (≤ 200 steps:
  a still tank, a falling block before impact), and on the dam break a new **`--statistics` metric for free-surface
  runs**: the front position x(t) at the frame times within one cell, the liquid mass within 1e-6 relative (both
  conserve it to rounding), and the mean kinetic energy within 1e-3 (the existing window statistic).
- Diagnostics reductions keep the f64 partials (ADR-020). The liquid mass Σ(m + massex) is summed as Σ(m + massex − φ)
  plus Σφ, both in f64 partials, so the f32 partial sums keep their digits.

## 9. Outputs (finding 7)

### 9.1 Frames, stream, samples

- `fillFraction` is produced whenever the free surface is on: φ from the `phi` plane (1 liquid, m/ρ interface, 0 gas
  and solid). `cfd_lbm::abi` gets the field index, the sampling kernels bind `surf`, and the CPU reference
  `Sample::from_lattice` handles it. **Proposed:** `Domain::from_scene` adds `FillFraction` to the output fields of a
  free-surface scene if the scene omits it (with an info issue), because the viewer needs it to mask the gas (§12).
- **Gas cells: velocity 0, pressure 0** (gauge = atmosphere), density 0, vorticity 0. This is the solid convention, so
  the box filter and the f16 encoding are unchanged. Interface cells report their values as they are.
- `LatticeFields` gains `fill: Option<Vec<f32>>` (φ) and `phase: Option<Vec<u8>>` (G/I/L, for vorticity's
  fluid-neighbour test and for probes). Vorticity uses liquid and interface neighbours only, the same rule as walls.
- Downsampling (k × k blocks): the viewer divides block means by the block's mean φ, the analogue of the solid
  fluid-fraction correction of Phase 5. It is approximate at the surface, because interface cells count fully in u but
  fractionally in φ. The dam break (202 × 101) is never downsampled at the default `maxSize` 1024.

### 9.2 Diagnostics and probes

- `diagnostics.csv`: `mass_lu` becomes the **liquid mass** Σ(m + massex) for free-surface runs (today Σρ over the
  static fluid cells, which would include gas). New columns `liquid_volume_lu` (Σφ), `interface_cells` and
  `mass_drift` (relative to step 0). Kinetic energy and max |u| over liquid and interface cells. The `series` hello
  message (ADR-031) carries the new header, so the plots pick it up.
- **Mass criterion:** |drift| < 0.1 % over the run (08 §4.6). In exact arithmetic the scheme conserves mass exactly,
  so the observed drift measures f32 rounding and is expected around 1e-6. 0.1 % is loose, which leaves room for the
  volume/compressibility confusion it was probably written to cover.
- **Probes** gain a `fill` column. In gas they report u = 0, p = 0 and fill 0 (so `probes.csv` keeps fixed columns,
  and "dry" is readable). The dam break's probe "Right wall" at (199, 5) then shows the impact as fill 0 → 1 and a
  pressure peak.
- Forces on walls (momentum exchange) are unchanged. Gas cells have no links, so a wall's force is the liquid's, gauge
  relative to the gas.

## 10. Validation design (finding 8)

### 10.1 Order (each a `cfd-verify` case, backend-agnostic, quick and full resolutions)

1. **Still water** (hydrostatic tank): a closed tank half full, gravity, 2 s. Checks: max |u| < 1e-3 √(g h) (spurious
   currents), the bottom pressure within 1 % of ρ₀ g h, the surface stays within ±0.5 cell of its height, mass drift
   < 1e-6, no conversions after the initial transient. A surface placed mid-cell (φ = 0.5 in the top row) tests the
   interface cells at rest.
2. **Free-falling block:** a square of liquid in a gas-filled box, gravity, until just before it reaches the floor.
   Exact solution: the block falls rigidly, its centroid at `y₀ − ½ g t²`, uniform velocity −g t, shape unchanged (no
   pressure gradients in free fall). Checks: centroid error < 1 % of the fall distance, internal velocity spread
   < 1 % of g t, mass drift < 1e-6. This isolates the body force, the reconstruction with a moving interface, and
   conversion at the leading and trailing faces.
3. **Dam break** (Martin & Moyce 1952), §10.3.

### 10.2 Tolerances fixed before the results (ADR-018)

| Case | Quick (CI, CPU) | Full |
|------|-----------------|------|
| Still water | as above, h = 20 cells | h = 40 cells |
| Falling block | 16-cell block | 32-cell block |
| Dam break: front Z(T) | 10 % at a = 20 cells | **5 % at a = 40 cells** (the example's resolution) |
| Dam break: liquid mass | < 0.1 % | < 0.1 % |

### 10.3 The dam break

- **Front:** `x_f` = the right end of the liquid connected to the left wall in the first fluid row (y = 1): scan from
  the wall while φ ≥ ½, then interpolate linearly between the last cell centre with φ ≥ ½ and the next one, to the
  point where φ = ½. Detached drops ahead of the tip are ignored. `Z = x_f / a`, a = 0.1 m measured from the wall face.
- **Time:** Martin & Moyce plot Z against `T = n t √(g/a)` with n² = H/a, which is `T = t √(2g/a)` for the n² = 2
  column (factor 14.0 s⁻¹ here). The front reaches the far wall (Z = 5) near T ≈ 3.3, t ≈ 0.24 s, so the comparison
  covers t ≤ 0.24 s of the 1 s run. The rest is the splash, for the viewer.
- **Reference data:** digitised from the paper's n² = 2 series into `validation/references/martin-moyce-1952.csv`,
  with the source, the figure/table and the digitising method in the header (as for Ghia and de Vahl Davis). If the
  paper stays out of reach, a secondary source that tabulates the same points (Koshizuka & Oka 1996 is a candidate,
  not yet checked) is cited as such. **Nothing is typed from memory.**
- **Early times:** the experiment's time origin is uncertain (the gate is removed in a finite time), and published
  simulations lead the experimental front at small T. **Proposed:** the 5 % is judged on the points with T ≥ 1;
  earlier points are shown in the report. No time shift is fitted.
- **Sanity check:** inviscid shallow-water (Ritter) theory gives the front speed 2√(gH), i.e. `dZ/dT → 2`. The measured
  slope must stay below it.
- Report: `validation/report*/` gains the Z(T) plot with the reference points, the mass-drift curve, and snapshots of
  φ at T = 1, 2, 3.

## 11. Scope (finding 9)

Each excluded combination stays a clear validation error with its own code:

| Combination | MVP | Error code |
|-------------|-----|------------|
| Free surface + inlets or outlets (velocity, pressure, zero-gradient) | **out**: mass sources and sinks, inflow of gas, an outlet's reference pressure against the gas pressure | `support.freeSurfaceOpen` |
| Free surface + thermal (and Boussinesq) | **out** | `support.freeSurfaceThermal` |
| Several liquids | **out** (already `support.multiFluid`) | unchanged |
| Surface tension, contact angle | **out** (6b) | n/a (no scene field yet) |
| Moving walls | allowed, not validated (bounce-back with u_w as today; wall links carry no mass exchange) | warning `support.freeSurfaceMovingWall` |
| Periodic edges | allowed (the passes use the same wrapped neighbour indexing); covered by one parity case | — |
| Initial velocity, no gravity | allowed (a moving drop) | — |
| Free surface on, but no *Empty* cell | allowed: a closed tank full of liquid, gravity as a body force | info |
| `free_surface` off with *Empty* cells | unchanged (`validate.rs:288`) | unchanged |
| Interactive patching (5b) of a free-surface run | later | — |
| A gas phase with its own inertia (water/air at density ratio 1000) | 6b, phase-field | — |

## 12. Editor and viewer (finding 10)

### 12.1 Editor (#46)

The palette already has *Fluid* and *Empty*. With *Free surface* on, new cells start empty (`store.ts:145`). So:

- **"Initial liquid region"** = cells painted *Fluid* in a free-surface scene. **Proposed:** in a free-surface scene the
  palette entry is labelled *Liquid* (same cell type, same data), with water colouring. No new cell type.
- **"Fill level"** = a tool *Fill to level*: click a height inside a container, and every *Empty* cell connected to the
  click (4-neighbour flood fill, bounded by solids and the domain edges) at or below that height becomes *Liquid*. One
  undo entry (the Phase 4 tile-swap history). The status bar shows the level in metres.
- **"Choice of liquid"** = the scene's initial fluid, picked from the library presets that are liquids (water, engine
  oil, glycerol, ethanol). The Scene tab shows τ for the choice (water: 0.50003 with the LES hint, glycerol 0.536).
  One liquid per scene (§11).
- The initial interface layer is not drawn by the user: `Domain::from_scene` makes the outermost *Liquid* cells
  (those with an *Empty* neighbour) interface cells with **φ = 1**. Then the liquid volume equals the painted cells
  exactly, and the φ = ½ contour of the first frame lies on the painted edge. FluidX3D instead turns the *gas* neighbours
  into φ = 0.5 interface cells, which adds half a cell of liquid around the region (+1.9 % volume for the dam-break
  column).

### 12.2 Viewer (#47)

- **Mask:** solids from the scene (as today), gas from the frame's `fillFraction` (cells with φ < ½ drawn as
  background). The colour map shows the chosen field on the liquid only.
- **Interface contour:** marching squares at φ = ½ over the frame's fill field (in the decode worker, like the
  streamlines; ≈ 20 k cells → < 1 ms), drawn as line segments over the field. The ambiguous saddle cases are resolved
  by the cell's mean.
- **Water colouring:** a *Liquid* display mode: a blue tint with opacity from φ (bilinear), optionally shaded by |u|.
  It is the default field for free-surface runs, with the contour on.
- Streamlines, particles and dye stop in gas (φ < ½), as they stop at solids.
- **Gallery:** enable the dam break (`ExampleGallery.tsx:142`) once the server runs it. **Quick variant (proposed):**
  dx = 5 mm (102 × 51 cells, a 20 × 40-cell column), `endTime` 0.5 s (the front reaches the wall at 0.24 s): 3,961
  steps × 5.2 k cells ≈ 2 × 10⁷ cell updates, about a second on the CPU even at half the single-phase speed. The full
  example is 3.2 × 10⁸ updates, an estimated 5–10 s on the CPU, which is borderline for the ADR-009 target. The quick
  variant is checked against Martin & Moyce at its own resolution (10 %, §10.2).

## 13. Code changes per crate (summary)

| Crate | Change |
|-------|--------|
| `cfd-core` | `scene`: `physics.turbulence` (optional). `domain`: drop `support.freeSurface`; new scope errors (§11); the free-fall velocity (§5); `FreeSurface { initial_phase, initial_fill }` in `Domain`; Empty → static FLUID; `Hydrostatic::gravity = 0` and `body_force = g_lb` in free-surface mode. `units`: τ check aware of the LES model. `solver`: `LatticeFields::{fill, phase}`, `Diagnostics::liquid` (volume, interface cells), φ output, gas masking, probe fill. `sample`: `FillFraction`. `examples`: dam break with `turbulence`, `quick("dam-break")`, `QUICK_NAMES` |
| `cfd-lbm` | `abi`: state bits, `surf` plane offsets, LES constant in `KernelParams`, the fill sample field. `kernel`: free-surface variant of `update_block` (pass 1) plus passes 2–4, a const generic `FREE_SURFACE` like `THERMAL` (≈ a C# generic specialised at compile time). `shared`: hydrostatic initial state, interface layer, diagnostics partials |
| `cfd-gpu` | `lbm_fs.wgsl` (pass 1) and `surface.wgsl` (passes 2–4), the `surf` buffer and bind groups, sampling of φ, reductions |
| `cfd-cuda` | `surface.cu` and a free-surface `stream_collide` entry point, graph capture of the four kernels, `static_assert`s from `abi` |
| `cfd-verify` | still water, falling block, dam break; free-surface parity cases; Martin & Moyce reference; the `--statistics` front metric |
| `cfd-job` | diagnostics and probe columns |
| `apps/web` | §12; generated types for `turbulence` |

## 14. Tests

- **Unit (CPU):** a single interface cell between liquid and gas conserves mass over one step to the last ulp; the
  exchange is antisymmetric (`Δm_k←j == −Δm_j←k` bit for bit); conversion rules on hand-made 5 × 5 states (IF, IG, GI,
  cancellation, lonely cells); excess accounting `Σ(m + massex)` is invariant over the four passes; the hydrostatic
  initial state; LES τ_eff against the closed formula; the invariants of §4.
- **Parity (`cfd_verify::parity`, shared by wgpu and CUDA):** still tank (100 steps), a falling block (200 steps,
  conversions included), a periodic wave tank, a small dam break (500 steps); per pass as well, with a test entry point
  that runs one pass on a given state, so a mismatch names the pass. CUDA bit-identical; wgpu 1e-5 and identical
  states on the short cases.
- **Validation:** §10, quick in CI (CPU), full locally (GPU), reports committed.
- **Compare:** `cfd-cli compare dam-break.json` (CUDA `--tolerance 0`, wgpu with the front statistics).
- **Web:** the marching-squares contour on analytic fields (a circle's φ), the fill-to-level tool, the gas mask; a
  Playwright test that runs the quick dam break on the CI's CPU engine and checks that the contour moves right.

## 15. MVP acceptance criterion (proposed, #48)

From the gallery, open **Dam break**, Run on **CUDA** and then on **wgpu**. In the job view the column collapses live
with the contour and the water colouring. After the run, the statistics panel shows the front Z(T) against Martin &
Moyce, **within 5 % for T ≥ 1**, and the **liquid-mass drift below 0.1 %**. Export a PNG, `diagnostics.csv` and a
video. With the cylinder and heated-cavity runs of Phase 5, this closes the MVP line of the plan: "for gases *and
liquids*, with validated results". A `@full` Playwright test covers the GPU part (as in Phase 5). The CPU quick variant
runs in CI.

## 16. Proposed order (each step tested and committed before the next)

0. Housekeeping: this proposal's answers, the ADR, 04 §5, 08 §4.6, the plan, and the 6a issues adjusted.
1. **#43 CPU solver:** scope errors and the free-fall time step; `Domain` free-surface data and the interface layer;
   the four passes and the hydrostatic start; still water and falling block passing; then the LES option; the dam
   break with options A and B of §7, measured (Q4 confirmed with numbers).
2. **#43 outputs:** φ in frames, gas masking, diagnostics and probe columns.
3. **#45 validation:** the Martin & Moyce reference, the three cases in `cfd-verify`, the CPU report.
4. **#44 GPU:** CUDA first (the bit-identical check finds kernel bugs fastest), then wgpu; parity cases; `compare`
   with the front statistics; GPU reports.
5. **#46 editor:** the *Liquid* label, fill to level, liquid choice and τ display.
6. **#47 viewer:** gas mask, contour, *Liquid* display mode, gallery and the quick variant.
7. **#48 acceptance:** the `@full` run, docs (02, 03, 04 §5, 08), the plan, the final check.

## 17. Open questions

| # | Question | Recommendation |
|---|----------|----------------|
| Q1 | Base algorithm: FluidX3D's four-pass structure (with `surface_0` folded into our pull), the conversion rules of §2.4 without the κ offset, a CPU that runs the same passes (no sequential lists)? (§1, §2.4, §4) | Yes. κ as an internal constant, 0 by default, raised only if the hydrostatic test shows flipping |
| Q2 | Gas reconstruction: only links from gas cells (FluidX3D), or also the links with `n·c_ī > 0` (Körner/Thürey, needs the normal)? (§2.3) | Gas links only; the normal-based rule as the fallback if #43 shows surface noise |
| Q3 | Excess mass: A (equal split over liquid and interface neighbours, FluidX3D), B (normal-weighted over interface neighbours, Thürey) or C (equal over interface neighbours)? Stored undivided with a receiver count, so the total mass is exact? (§2.5) | A, stored undivided with the count |
| Q4 | Viscosity at τ₀ = 0.50003: Smagorinsky LES as an optional `physics.turbulence` scene field (A), an effective viscosity (B), or both measured on the dam break before choosing? (§7) | A, with B measured alongside in #43; the τ check becomes a warning only when a model is set |
| Q5 | Smagorinsky constant default, if Q4 = A? (§7) | C_s = 0.1, adjustable in the scene; revisit after the front comparison |
| Q6 | Time step: add the free-fall velocity √(2 g H_l) to the characteristic velocities, mapped to the default u_lb = 0.05 (dt = 6.31e-5 s, front at Ma 0.12), rather than 0.1? (§5) | Yes, 0.05 |
| Q7 | Gravity: a body force (Guo) on the liquid with the hydrostatic split off (ADR-016 amended), the output pressure as gauge relative to the gas, a hydrostatic initial density? (§6) | Yes to all three |
| Q8 | Data layout: one `surf` buffer of five planes (state ×2 ping-pong `u32`, mass, massex, φ), 114 B/cell, bound in place of `g_src` on wgpu so the kernel keeps 8 storage buffers? (§3) | Yes |
| Q9 | ρ stored every step on liquid and interface cells in free-surface mode, u on interface cells (a partial undo of `skip_macro` for these runs)? (§3.1) | Yes; revisit in 8a |
| Q10 | Parity: CUDA bit-identical (`--tolerance 0`), wgpu at 1e-5 on short cases and a new free-surface `--statistics` metric (front within one cell, mass 1e-6, kinetic energy 1e-3) on long runs? (§8) | Yes |
| Q11 | Outputs: gas cells write u = p = 0 (p = 0 is the atmosphere); `fillFraction` added automatically to free-surface scenes; diagnostics report the liquid mass Σ(m + massex), volume Σφ and interface count; probes gain a `fill` column? (§9) | Yes |
| Q12 | Validation: still water and the falling block before the dam break, with the tolerances of §10.2 fixed now; the front as the φ = ½ crossing in the first fluid row; the 5 % judged on T ≥ 1; data digitised from the paper (or a cited secondary source), never from memory? (§10) | Yes |
| Q13 | Scope: free surface with inlets/outlets and with thermal out of the MVP as errors; moving walls allowed with a warning; periodic edges allowed? (§11) | Yes |
| Q14 | Initial interface: the outermost painted liquid cells become interface cells with φ = 1 (volume exact), rather than FluidX3D's φ = 0.5 gas neighbours (+1.9 % volume)? (§12.1) | φ = 1 |
| Q15 | Editor: *Liquid* as the label of *Fluid* in free-surface scenes, a flood-fill *Fill to level* tool, the liquid chosen from the library's liquid presets with τ shown? (§12.1) | Yes |
| Q16 | Viewer: gas masked by φ < ½, the φ = ½ contour by marching squares in the worker, a *Liquid* display mode as the default for free-surface runs? (§12.2) | Yes |
| Q17 | Dam break quick variant: dx = 5 mm, `endTime` 0.5 s, judged at 10 % against Martin & Moyce at its own resolution? Keep the full example at `endTime` 1 s? (§12.2) | Yes to both |
| Q18 | Acceptance: the dam break on CUDA and wgpu from the gallery, front within 5 % for T ≥ 1, mass drift < 0.1 %, PNG/CSV/video export, a `@full` Playwright test? (§15) | Yes |
