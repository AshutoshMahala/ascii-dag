# Benchmarks

Numbers are grouped by release, newest first — the same convention as
the changelog. A section is only ever labelled with the version it was
actually measured on; nothing is carried forward silently.

## How to run

```bash
cargo run --release --example benchmark --features arena
```

```bash
cargo run --release --example stress_test --features arena -- --csr
```

```bash
cargo run --release --example content_overhead --features arena
```

Desktop figures: Apple M2 Ultra (ARM64), release build. "Heap" is the
default `Graph` pipeline; "Arena" is the CSR/no-alloc pipeline
(`--features arena`). Embedded figures come from physical hardware —
the crates under `examples/` build and run on the boards named.

---

# 0.11 (unreleased)

Measured on 2026-09-06 from the 0.11 development tree at `f36bbf1`
(`main`, as identified by the changelog; the Cargo package version
has not yet been bumped from `0.10.3`). Rust 1.94.0,
`aarch64-apple-darwin`, with the repository's release profile:
`opt-level = "z"`, LTO, one codegen unit, stripped, `panic = "abort"`.

All desktop runs use default features plus `arena`: both layout axes,
`ports`, `std` and `generic` are enabled. The measured graphs use
`TopDown`, the standard layout configuration, plain rendering, and no
declared ports. Timings are medians of five consecutive runs after
warm-up, with benchmark processes run sequentially. Each timing column
is aggregated independently; totals are measured directly, so they need
not equal the sum of the phase medians. Speedup is the ratio of median
heap total to median arena total.

## Desktop (Apple M2 Ultra, ARM64, release)

| Topology | Nodes | Mode | Build | Compute | Render | **Total** | Speedup |
| :--- | ---: | :--- | ---: | ---: | ---: | ---: | ---: |
| **Chain** | 100 | Heap | 24µs | 249µs | 46µs | **321µs** | |
| | | Arena | 3µs | 45µs | 56µs | **105µs** | **3.1x** |
| **Chain** | 250 | Heap | 53µs | 559µs | 111µs | **721µs** | |
| | | Arena | 5µs | 92µs | 146µs | **243µs** | **3.0x** |
| **Diamond** | 100 | Heap | 27µs | 916µs | 81µs | **1031µs** | |
| | | Arena | 3µs | 196µs | 114µs | **314µs** | **3.3x** |
| **Diamond** | 200 | Heap | 61µs | 1813µs | 173µs | **2051µs** | |
| | | Arena | 5µs | 401µs | 241µs | **648µs** | **3.2x** |
| **WideFan** | 100 | Heap | 28µs | 428µs | 163µs | **621µs** | |
| | | Arena | 3µs | 56µs | 220µs | **283µs** | **2.2x** |
| **WideFan** | 500 | Heap | 156µs | 2124µs | 948µs | **3208µs** | |
| | | Arena | 14µs | 292µs | 1165µs | **1467µs** | **2.2x** |

`examples/benchmark.rs`: Chain is linear, Diamond adds skip-level
edges, and WideFan fans out from a root and back into a sink. Arena
backing buffers for graph construction and layout are allocated before
timing; render workspace and output-buffer allocation are included in
the Render phase.

## Scalability (stress shapes)

`examples/stress_test.rs`, without arguments for heap and with `--csr`
for arena. Graph construction and CSR conversion are outside these
timings; render-buffer allocation is included. Output sizes below are
bytes, identical in length on both pipelines in every recorded run.

| Topology | Nodes | Mode | Time | Output |
| :--- | ---: | :--- | ---: | ---: |
| **Massive Diamond** | 20,164 | Heap layout+render | 137.2 ms | 639,866 B |
| | | Arena layout+render | **32.9 ms** | 639,866 B |
| **Massive Diamond** | 50,176 | Heap layout+render | 398.0 ms | 1,597,134 B |
| | | Arena layout+render | **86.1 ms** | 1,597,134 B |
| **Massive Fan** | 50,000 | Heap layout+render | 623.6 ms | 6,100,374 B |
| | | Arena layout+render | **137.1 ms** | 6,100,374 B |

## Node-content storage overhead

`examples/content_overhead.rs`: a 10,000-node chain per kind, with heap
layout+render timings and the public arena-size estimates. Estimates
are provisioning bounds, not measured peak RAM.

| Kind | Build | Layout+render | CSR estimate | Layout estimate |
| :--- | ---: | ---: | ---: | ---: |
| all simple | 2.9 ms | 29.0 ms | 880,296 B | 7,115,158 B |
| all boxed | 3.0 ms | 34.9 ms | 880,296 B | 7,115,158 B |
| all custom (painter + 8 B payload) | 3.1 ms | 32.0 ms | 1,280,312 B | 7,515,174 B |

Boxed nodes add **0.0 B/node** to either estimate. Custom nodes add
**40.0 B/node** (400,016 B total per estimate, including fixed overhead).

## Bundle size (WASM)

Same method as the 0.10.0 section below: a minimal `cdylib` consumer
that builds an *n*-node chain, lays it out and renders it, `n` a
runtime argument; crate release profile (`opt-level = "z"`, LTO,
`codegen-units = 1`, `strip`, `panic = "abort"`), then
`wasm-opt -Oz --all-features` (binaryen 130) and `gzip -9`. The
baseline is **v0.10.2 rebuilt through this same harness** on the same
day, so the deltas are like-for-like. The 0.10 column in the 0.10.0
section came from an earlier harness and reads lower (200.2 KB
default, 94.2 KB arena) than v0.10.2 measures here; compare within a
section, not across them.

| Configuration | v0.10.2 | 0.11 | Delta |
| :--- | ---: | ---: | ---: |
| `arena` (no-alloc), default features | 125,354 B | **153,935 B** | **+22.8%** |
| | 53,541 B gz | 65,933 B gz | +23.1% |
| `arena`, without `ports` | — | 129,285 B | +3.1% |
| | | 54,722 B gz | +2.2% |
| default (`std` + `generic`), default features | 259,326 B | **298,310 B** | **+15.0%** |
| | 102,798 B gz | 119,618 B gz | +16.4% |
| default, without `ports` | — | 261,307 B | +0.8% |
| | | 104,535 B gz | +1.7% |

**Almost all of 0.11's growth is the `ports` default feature.** With
it off, the 0.11 module is within 1–3% of v0.10.2 in both
configurations. `ports` costs 24,650 B on the arena build (+19.1%)
and 37,003 B on the default build (+14.2%), `wasm-opt`'d; that is the
same feature whose absence keeps the Longan Nano firmware inside its
flash (above).

Splitting by stage, the same harness with rendering removed:

| Stage | v0.10.2 | 0.11 (default features) | Delta |
| :--- | ---: | ---: | ---: |
| `arena`, layout only | 80,715 B | 105,758 B | +31.0% |
| `arena`, render on top | 44,639 B | 48,177 B | +7.9% |
| default, layout only | 202,291 B | 242,344 B | +19.8% |
| default, render on top | 57,035 B | 55,966 B | −1.9% |

The layout-stage growth is where `ports` lives (its placement runs
in layout); the render stage is flat to slightly smaller despite the
scene planner, composer and terminal renderer landing in 0.11. Stage
figures are subtraction under LTO, so read them as apportionment.

### Layout-axis selection

`layout-vertical` / `layout-horizontal` gate the two monomorphized
layout profiles. Measured on this tree with default features
(`ports` on), which is why these figures differ from the
axis-selection table recorded under 0.10.0, measured on a development
tree before `ports` existed.

| Configuration | wasm-opt | gzip -9 |
| :--- | ---: | ---: |
| both axes, layout + render | 298,310 B | 119,618 B |
| `layout-vertical` only | **234,960 B (−21.2%)** | **94,063 B (−21.4%)** |
| `layout-horizontal` only | **236,094 B (−20.9%)** | **94,484 B (−21.0%)** |
| both axes, layout only | 242,344 B | 96,591 B |
| `layout-vertical` only | **179,085 B (−26.1%)** | **70,801 B (−26.7%)** |
| `layout-horizontal` only | **180,227 B (−25.6%)** | **71,333 B (−26.1%)** |

The horizontal profile is ~1.1 KB larger than the vertical one, in
line with the 0.10-era measurement.

## Embedded: RP2040 Pico (Cortex-M0+, 125 MHz, 264 KB SRAM)

`examples/rp2040_pico`, rebuilt against this tree (`alloc`,
`arena-idx-u8`, `layout-vertical`; no `ports`; `opt-level = "s"`,
LTO) and flashed over UF2; firmware `.text` 326,884 B. The benchmark
source is unchanged from 0.10.2, so the rows below use the same
measurement procedure as the 0.10.0 rows further down, with the
library rebuilt from this tree. It runs each chain
through both pipelines and reports over USB serial. RAM is the
heap-in-use delta across the run for the heap mode, and layout
temp-arena bytes used for the arena mode. Two boots were captured;
every cell agreed within about 1%, and the first is shown.

| Graph | Nodes | Mode | Build | Compute | Render | **Total** | RAM | Speedup |
| :--- | ---: | :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| **Chain 10** | 10 | Heap | 0.45 ms | 3.41 ms | 1.30 ms | **5.16 ms** | 4,032 B | |
| | | Arena | 0.33 ms | 1.79 ms | 1.78 ms | **3.90 ms** | **1,864 B** | **1.3x** |
| **Chain 50** | 50 | Heap | 1.61 ms | 13.62 ms | 4.05 ms | **19.27 ms** | 18,832 B | |
| | | Arena | 0.75 ms | 3.57 ms | 6.72 ms | **11.04 ms** | **8,904 B** | **1.7x** |
| **Chain 100** | 100 | Heap | 3.14 ms | 27.93 ms | 7.15 ms | **38.22 ms** | 37,584 B | |
| | | Arena | 1.59 ms | 5.83 ms | 12.78 ms | **20.19 ms** | **17,700 B** | **1.9x** |

Against the 0.10.0 rows below, on the same board: **the heap path is
unchanged** — every total is within 0.02 ms, and heap RAM is up by
0.3–0.5 KB at 50 and 100 nodes. **The arena path is slower**, and the
regression is in render: 1.52 → 1.78 ms (+17%) at Chain 10,
5.24 → 6.72 ms (+28%) at Chain 50, 9.71 → 12.78 ms (+32%) at
Chain 100. Arena compute is +7–9% and arena build is +0.01–0.10 ms;
arena RAM matches 0.10.0 to the 0.1 KB it recorded. End to end the arena path is
+11% / +20% / +22%, which pulls the heap-over-arena speedup down from
1.5× / 2.1× / 2.3× to 1.3× / 1.7× / 1.9×. Arena render on this core
now costs more than heap render at every size measured; at 100 nodes
it is 1.8× the heap figure.

## Embedded: ESP32-S3 (Xtensa LX7, 80 MHz configured, 512 KB SRAM)

`examples/esp32s3`, rebuilt against this tree and flashed to an
ESP32-S3 (silicon revision v0.2) with the `esp` toolchain and
espflash 4.3.0; app image 280,672 B. The crate enables `alloc`,
`arena-idx-u8` and `layout-vertical` only — no `ports` — and runs the
heap pipeline on a 128 KB `embedded-alloc` heap. "Render" is a single
`Graph::render()` call, so it covers the cycle check, layout and
paint together. RAM is live heap after the call, not peak. Single
run; the monitor captured two boots, and every shape that ran on both
agreed to the microsecond.

Clock metadata correction: the checked-in firmware uses
`esp_hal::Config::default()`, which selects 80 MHz in the locked
esp-hal 1.0.0 dependency. Its printed 240 MHz banner does not match
that configuration. The recorded timings below are unchanged; they
should not be interpreted as measurements at 240 MHz.

| Graph | Nodes | Edges | Build | Render | RAM |
| :--- | ---: | ---: | ---: | ---: | ---: |
| **Diamond** | 4 | 4 | 0.62 ms | 5.82 ms | 1,536 B |
| **Build Pipeline** | 10 | 12 | 0.74 ms | 12.56 ms | 3,592 B |
| **Fan-Out/Fan-In** | 12 | 16 | 0.91 ms | 11.08 ms | 4,792 B |
| **Binary Tree** | 31 | 30 | 1.35 ms | 18.98 ms | 11,920 B |
| **Deep Chain** † | 50 | 49 | 2.20 ms | 3.34 ms | 20,312 B |
| **Diamond Lattice** | 64 | 112 | 2.96 ms | 52.89 ms | 26,816 B |

† Simple-chain shortcut: `Graph::render()` emits a single-component
graph whose nodes each have at most one parent and one child as
inline `[A] → [B]` text without entering the layout pipeline, so this
row measures a different code path from the other five.

Against the 0.10.0 rows below, on the same board: **live heap matches
within the 0.1 KB precision reported there** on all six shapes; the
rounded historical values cannot establish byte-for-byte equality.
Render on the five pipeline
shapes is within +1% to +5% on Diamond, Binary Tree and Diamond
Lattice, and +12% / +19% on Fan-Out/Fan-In and Build Pipeline; the
chain shortcut is −2%. Build is 0.05–0.07 ms slower on the five
pipeline shapes and flat (+0.01 ms) on the chain.
These are observed differences, not a statistical noise bound. A
single sample per release cannot establish the significance or cause
of the mid-size shapes' increases.

## Embedded: Longan Nano (GD32VF103, RISC-V, 128 KB flash / 32 KB RAM)

**The measured 0.11 demo configuration did not complete on this board.**
`examples/longan_nano` built and fit flash, but the firmware halted
inside `compute_layout_arena` on the hardware: the title line drew,
then nothing. Diagnosed with
progress markers on the LCD and a frame scan of the ELF; the numbers
below are from this tree at `f36bbf1` with the example's layout temp
buffer raised to 4,096 B (it ships at 2,048 B in 0.10.x). That build
used separate graph (1,024 B), layout output (2,048 B), layout temp
(4,096 B), render (4,096 B) and text (2,048 B) buffers. These results
predate the workspace-reuse update to the example; they are not
measurements of that update.

| Measure | v0.10.0 | v0.10.2 | 0.11 |
| :--- | ---: | ---: | ---: |
| Firmware `.text` | — | 112,870 B | 114,918 B |
| Flashed image (`.bin`) | — | 114,828 B | 116,832 B (of 131,072) |
| Layout temp arena needed, demo graph | 856 B | **3,340 B** | 3,340 B |
| Layout output arena needed | 608 B | 608 B | 624 B |
| Graph arena needed | 368 B | 368 B | 368 B |
| Render arena estimate / output estimate | 1,360 / 912 B | 1,336 / 888 B | 1,304 / 888 B |
| Layout function stack frame | — | 16,016 B | 16,160 B |
| `main` stack frame (demo buffers + locals) | — | 11,744 B | 14,832 B |

Arena minimums are exact, found by bisection on a 32-bit build
(`wasm32`, the same pointer width as the RISC-V target; a 64-bit host
overstates them). The stack rows preserve the original frame-scan
readings from the unstripped ELF, but those readings omitted shared
outlined prologues and are partial frame sizes, not total stack use.
The 92,530 B `.text` recorded in the 0.10.0
section below does not reproduce at v0.10.2, which builds at
112,870 B; that figure predates the 0.10.x patches.

**Two limits in the measured demo configurations:**

- **The demo's arena budget was outgrown in 0.10.1.** The skip-level
  edge router (chain-lane allocator, commit `665577e`, shipped in
  0.10.1) raised the layout temp arena the demo graph needs from
  856 B to 3,340 B; the demo has one skip-level edge. The example's
  2,048 B buffer therefore returns `ArenaOom` on 0.10.1 and 0.10.2 —
  reported cleanly, as the 0.10.1 changelog said it would, but the
  board was not re-flashed after 0.10.0 and the demo's hardcoded size
  was never re-run against the estimate.
- **The no-alloc layout keeps ~16 KB of scratch on the stack.**
  Fixed 512-entry arrays in the cluster and subgraph compaction passes
  (`bodies`, `members`, `by_dist`) are inlined into the layout
  function and sit outside the arena estimate. They are the same size
  under `arena-idx-u8`, where the graph is capped at 255 nodes. On
  this part almost all 32 KB of SRAM is available to the stack, but
  the separate demo buffers, layout frame and nested callees together
  exceed it. Stack exhaustion is outside the arena's `ArenaOom`
  reporting contract.

Stack-accounting correction: in the inspected 0.11 ELF, both `main`
and layout also call a shared prologue that reserves stack before
their local frame adjustments; nested callees must be counted too.
The runtime's default per-hart stack allotment is not an additional
startup frame on hart 0. The earlier derived totals and headroom
claims therefore should not be used as stack high-water measurements.
The recorded frame-scan values above remain unchanged for provenance.

The current example keeps horizontal layout only, without `ports`,
and reuses its layout temp workspace for render scratch and text once
layout completes. It removes the separate render and text buffers;
a fresh hardware run is still needed to validate the updated demo.

Enabling `ports` in the measured configuration did not link: `.text`
overflowed the flash region by 3,814 B.

---

# 0.10.0

## Desktop (Apple M2 Ultra, ARM64, release)

| Topology | Nodes | Mode | Build | Compute | Render | **Total** | Speedup |
| :--- | ---: | :--- | ---: | ---: | ---: | ---: | ---: |
| **Chain** | 100 | Heap | 70µs | 585µs | 90µs | **746µs** | |
| | | Arena | 6µs | 107µs | 91µs | **206µs** | **3.6x** |
| **Chain** | 250 | Heap | 128µs | 1078µs | 170µs | **1377µs** | |
| | | Arena | 9µs | 131µs | 217µs | **358µs** | **3.8x** |
| **Diamond** | 100 | Heap | 59µs | 1567µs | 168µs | **1795µs** | |
| | | Arena | 5µs | 298µs | 219µs | **523µs** | **3.4x** |
| **Diamond** | 200 | Heap | 125µs | 2957µs | 209µs | **3292µs** | |
| | | Arena | 7µs | 266µs | 272µs | **545µs** | **6.0x** |
| **WideFan** | 100 | Heap | 31µs | 403µs | 173µs | **608µs** | |
| | | Arena | 3µs | 53µs | 219µs | **276µs** | **2.2x** |
| **WideFan** | 500 | Heap | 142µs | 2035µs | 1015µs | **3193µs** | |
| | | Arena | 14µs | 393µs | 1232µs | **1641µs** | **1.9x** |

*Chain = linear (best case), Diamond = skip-level edges (stress),
WideFan = fan-out/fan-in (crossing worst case).*

## Scalability (stress shapes)

| Topology | Nodes | Mode | Time | Output |
| :--- | ---: | :--- | ---: | ---: |
| **Massive Diamond** | 20,164 | Heap layout+render | 146 ms | 0.61 MB |
| | | Arena layout+render | **30 ms** | |
| **Massive Diamond** | 50,176 | Heap layout+render | 419 ms | 1.52 MB |
| | | Arena layout+render | **83 ms** | |
| **Massive Fan** | 50,000 | Heap layout+render | 632 ms | 5.82 MB |
| | | Arena layout+render | **340 ms** | |

Against the same shapes on 0.9.x (below), that is 3.1× on Diamond 20k,
5.7× on Diamond 50k, and **28× on Fan 50k** — the last from the
engine's merged-run painting, which stopped repainting overlapping
horizontal spans cell by cell.

## Node-content storage overhead

10,000-node chain, per kind, measured on the public arena-estimate
contract (what embedded users provision against):

| Kind | Build | Layout+render | CSR estimate | Layout estimate |
|---|---:|---:|---:|---:|
| all simple | 2.6 ms | 32.5 ms | baseline | baseline |
| all boxed | 2.6 ms | 33.3 ms | **+0.0 B/node** | **+0.0 B/node** |
| all custom (painter + 8 B payload) | 2.5 ms | 28.6 ms | **+40.0 B/node** | **+40.0 B/node** |

Sparse + packed storage: the built-in kinds cost nothing (the kind tag
packs into existing per-node flags), and only nodes declaring a
painter or payload get a side-table entry — 32 B plus payload bytes.
The timings show no systematic difference between the kinds; the
spread above is run-to-run noise (the custom row coming out fastest is
the giveaway).

## What the rank directions cost

All four directions share **one** layout pipeline, parameterized by a
zero-sized axis profile the compiler monomorphizes away. Both profiles
(`Vertical` for TB/BT, `Horizontal` for LR/RL) are reachable from
`compute_layout()`, so the binary carries two stamped copies of the
layout stage. Release build, stripped, against the pre-direction
baseline:

| Binary | Before | After | Delta |
|---|---:|---:|---:|
| `hero` example | 502,552 B | 585,976 B | **+16.6%** |
| `benchmark` example | 519,048 B | 585,800 B | **+12.9%** |

Layout time is a wash (min-of-3 interleaved: −11% to +20%, no
systematic direction). The two profiles execute the same algorithm —
TD/BT output is byte-identical before and after — so the variation is
scheduling and instruction-cache pressure from the larger binary, not
extra work.

## Bundle size (WASM)

Measured on a minimal `cdylib` consumer that builds an *n*-node
chain, lays it out and renders it, so the linker keeps what a real
caller pulls in and nothing more. `n` is a runtime argument, so the
work cannot be constant-folded away. Profile is the crate's own —
`opt-level = "z"`, LTO, `codegen-units = 1`, `strip`,
`panic = "abort"` — then `wasm-opt -Oz --all-features` and `gzip -9`.
Both versions are measured with the same harness doing the same
work through each version's API.

| Configuration | 0.9.1 | 0.10 | Delta |
| :--- | ---: | ---: | ---: |
| `arena` (no-alloc) | 46.4 KB | **94.2 KB** | **+103%** |
| | 20.2 KB gz | 40.6 KB gz | +101% |
| default (`std` + `generic`) | 95.3 KB | **200.2 KB** | **+110%** |
| | 40.0 KB gz | 80.5 KB gz | +101% |

**The bundle roughly doubled.** Splitting by stage — the same
harness with rendering removed, so the remainder is the render code
the linker keeps:

| Stage | 0.9.1 | 0.10 | Delta |
| :--- | ---: | ---: | ---: |
| `arena`, layout only | 40.3 KB | 60.1 KB | +49% |
| `arena`, render on top | 6.2 KB | 34.1 KB | +451% |
| default, layout only | 88.4 KB | 160.4 KB | +81% |
| default, render on top | 6.9 KB | 39.8 KB | +477% |

Two structural causes:

**Layout roughly doubled** because all four directions share one
pipeline parameterized by a zero-sized axis profile, and
`compute_layout()` can reach both profiles — so the module carries
two monomorphized copies. The same effect the native binaries show
above, but far more visible here, where layout is most of the
module. A cargo feature gating the `LeftRight`/`RightLeft` dispatch
arms would make the `Horizontal` copy strippable and return most of
this to callers who only need vertical layouts.

**Render grew about six-fold** because the engine replaced a single
scanline painter with semantic cells, two charset decode tables,
plan construction with a spatial index, banding, color planes,
styling, legend and hit-testing. That is the feature set rather than
overhead — but all of it is reachable from `render_string`, so a
caller who wants only plain monochrome output cannot currently strip
any of it.

Stage figures come from subtraction, and LTO shares code across
stages, so read them as apportionment rather than as independent
modules.

### Layout-axis selection (0.11)

The `layout-vertical` / `layout-horizontal` features gate the two
monomorphized layout profiles (and their `Direction` variants; both are
default features, at least one is required). Measured on the same
layout-only chain consumer and pipeline as above (`wasm-opt -Oz`,
`gzip -9`), `std` + `generic`:

| Configuration | wasm-opt | gzip -9 |
| :--- | ---: | ---: |
| both axes (default) | 200,180 B | 81,346 B |
| `layout-vertical` only | **150,760 B (−24.7%)** | **60,499 B (−25.6%)** |
| `layout-horizontal` only | **152,136 B (−24.0%)** | **61,087 B (−24.9%)** |

The axes cost nearly the same; the horizontal profile is ~1.4 KB
larger (its extra cross-axis envelope and label-extent handling).
Native (`aarch64-apple-darwin` cdylib, same profile): 468,928 →
418,608 B either axis (−10.7%; Mach-O page alignment makes the raw
sizes identical — gzipped they differ by ~1 KB, 199,284 vertical vs
200,222 horizontal), clean-build time −22%. The pre-implementation stub
measurement (dispatch hard-wired to one profile) predicted 150,537 B /
60,500 B — the real feature surface reproduces it within 0.15%
(one byte, gzipped), so the feature carries no measurable plumbing
overhead.

## Embedded: RP2040 Pico (Cortex-M0+, 125 MHz, 264 KB SRAM)

`examples/rp2040_pico` runs the same chain benchmark through both
pipelines and reports over USB serial. RAM is peak heap for the heap
mode, arena bytes for the arena mode.

| Graph | Nodes | Mode | Build | Compute | Render | **Total** | RAM | Speedup |
| :--- | ---: | :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| **Chain 10** | 10 | Heap | 0.45 ms | 3.46 ms | 1.28 ms | **5.18 ms** | 3.9 KB | |
| | | Arena | 0.32 ms | 1.67 ms | 1.52 ms | **3.50 ms** | **1.8 KB** | **1.5x** |
| **Chain 50** | 50 | Heap | 1.60 ms | 13.71 ms | 3.95 ms | **19.26 ms** | 18.1 KB | |
| | | Arena | 0.70 ms | 3.28 ms | 5.24 ms | **9.22 ms** | **8.7 KB** | **2.1x** |
| **Chain 100** | 100 | Heap | 3.13 ms | 28.04 ms | 7.06 ms | **38.23 ms** | 36.2 KB | |
| | | Arena | 1.49 ms | 5.34 ms | 9.71 ms | **16.54 ms** | **17.3 KB** | **2.3x** |

Against the 0.9.x rows below, measured on the same board: **build is
25–39% faster** on the heap path, and **peak heap RAM is down 22–23%**
(47.1 → 36.2 KB at Chain 100). **Render is slower**, by a margin that
shrinks as the graph grows — arena render is 3.0× the 0.9.x time at
Chain 10, 1.9× at Chain 50, 1.3× at Chain 100.

That shape is the 0.10 engine's tradeoff, not an embedded-specific
regression: `RenderPlan` construction is a larger fixed cost, and
painting from a plan has a flatter per-node slope, so the two curves
converge and then cross. The desktop table above shows the same
crossover — arena render is +44% against 0.9.x at Chain 100 and −40%
at Chain 250. The M0+ simply has not reached the crossing point by
100 nodes. End to end at Chain 100 the heap path is a wash
(38.0 → 38.2 ms) and the arena path is 10% slower.

## Embedded: ESP32-S3 (Xtensa LX7, 240 MHz, 512 KB SRAM)

`examples/esp32s3` runs six shapes through the heap pipeline
(`alloc` + `embedded-alloc`, 128 KB heap; no arena mode on this
board). "Render" is a single `Graph::render()` call, so it covers the
cycle check, layout and paint together — the same call the 0.9.x rows
below measured. RAM is live heap after the call, not peak.

| Graph | Nodes | Edges | Build | Render | RAM |
| :--- | ---: | ---: | ---: | ---: | ---: |
| **Diamond** | 4 | 4 | 0.55 ms | 5.58 ms | 1.5 KB |
| **Build Pipeline** | 10 | 12 | 0.68 ms | 10.53 ms | 3.5 KB |
| **Fan-Out/Fan-In** | 12 | 16 | 0.86 ms | 9.90 ms | 4.7 KB |
| **Binary Tree** | 31 | 30 | 1.29 ms | 18.20 ms | 11.6 KB |
| **Deep Chain** † | 50 | 49 | 2.19 ms | 3.41 ms | 19.8 KB |
| **Diamond Lattice** | 64 | 112 | 2.89 ms | 52.42 ms | 26.2 KB |

† `Graph::render()` takes a simple-chain shortcut: a single-component
graph whose nodes each have at most one parent and one child renders
as inline `[A] → [B]` text without entering the layout pipeline. This
row measures a different code path from the other five, and is not
comparable to the RP2040 chain figures above.

Against the 0.9.x rows below: RAM is essentially unchanged (0–3%
lower), and render on the five pipeline shapes is **16–57% slower**.
The chain row, on the shortcut path, is unchanged at +7%. The only
shape large enough to say much is Diamond Lattice, whose +16% is the
mildest of the five; the four small shapes cluster at +37% to +57%
without ordering cleanly by size, which is what six differently
shaped graphs at one sample each should be expected to look like.
Direction-consistent with the RP2040 fixed-cost signature on an
unrelated architecture, but nothing here is big enough to reach the
crossover. Build is flat to modestly slower, but the 0.9.x figures
were recorded to 0.1 ms, so the sub-millisecond rows cannot support
a precise delta. Single run, not min-of-N.

## Embedded: Longan Nano (GD32VF103, RISC-V, 128 KB flash / 32 KB RAM)

`examples/longan_nano` renders to the board's 160×80 LCD in
`LeftRight` with the ASCII charset, no allocator anywhere:

| Measure | Value |
|---|---:|
| Firmware `.text` | 92,530 B (of 128 KB flash) |
| Arenas (all on the stack) | ~10 KB (of 32 KB SRAM) |
| Render working set for the demo graph | 1,568 B arena + 912 B text |

Built with `opt-level = "z"` + LTO; the default release profile does
not fit.

---

# 0.9.x (historical)

Kept for comparison. **The 0.10 render engine invalidates several of
these** — the scalability rows above are the same shapes re-measured.

## Desktop (Apple M2 Ultra, ARM64, release)

| Topology | Nodes | Mode | Build | Compute | Render | **Total** | Speedup |
| :--- | ---: | :--- | ---: | ---: | ---: | ---: | ---: |
| **Chain** | 100 | Heap | 59µs | 526µs | 58µs | **645µs** | |
| | | Arena | 6µs | 79µs | 63µs | **148µs** | **4.4x** |
| **Chain** | 250 | Heap | 123µs | 1294µs | 205µs | **1623µs** | |
| | | Arena | 13µs | 389µs | 362µs | **765µs** | **2.1x** |
| **Diamond** | 100 | Heap | 78µs | 2267µs | 225µs | **2572µs** | |
| | | Arena | 6µs | 296µs | 127µs | **430µs** | **6.0x** |
| **Diamond** | 200 | Heap | 133µs | 3516µs | 378µs | **4027µs** | |
| | | Arena | 10µs | 679µs | 308µs | **998µs** | **4.0x** |
| **WideFan** | 100 | Heap | 55µs | 755µs | 263µs | **1075µs** | |
| | | Arena | 5µs | 55µs | 265µs | **326µs** | **3.3x** |
| **WideFan** | 500 | Heap | 276µs | 4166µs | 4639µs | **9082µs** | |
| | | Arena | 23µs | 241µs | 4µs | **268µs** | **33.9x** |

## Scalability (stress shapes)

| Topology | Nodes | Mode | Time | Output Size |
| :--- | ---: | :--- | ---: | ---: |
| **Diamond** | 20,164 | Heap | 450 ms | 0.61 MB |
| | | Arena | 994 ms | |
| **Diamond** | 50,176 | Heap | 2.4 s | 1.52 MB |
| | | Arena | 6.1 s | |
| **Wide Fan** | 50,000 | Heap | 18.0 s | 5.82 MB |
| | | Arena | 4.6 s | |

## Embedded (RP2040 / Cortex-M0+ @ 125 MHz)

| Graph | Nodes | Mode | Build | Compute | Render | **Total** | RAM | Speedup |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Chain 10** | 10 | Heap | 0.6 ms | 3.1 ms | 0.6 ms | **4.3 ms** | 5.0 KB | |
| | | Arena | 0.3 ms | 1.1 ms | 0.5 ms | **1.9 ms** | **1.9 KB** | **2.3x** |
| **Chain 50** | 50 | Heap | 2.5 ms | 13.2 ms | 2.4 ms | **18.2 ms** | 23.6 KB | |
| | | Arena | 0.6 ms | 2.9 ms | 2.7 ms | **6.2 ms** | **9.1 KB** | **3.0x** |
| **Chain 100** | 100 | Heap | 5.1 ms | 28.0 ms | 4.9 ms | **38.0 ms** | 47.1 KB | |
| | | Arena | 1.4 ms | 6.2 ms | 7.5 ms | **15.0 ms** | **18.2 KB** | **2.5x** |

## Embedded (ESP32-S3 / Xtensa LX7 @ 240 MHz)

| Graph | Nodes | Edges | Build | Render | RAM |
| :--- | ---: | ---: | ---: | ---: | ---: |
| **Diamond** | 4 | 4 | 0.5 ms | 3.7 ms | 1.5 KB |
| **Build Pipeline** | 10 | 12 | 0.5 ms | 7.7 ms | 3.6 KB |
| **Fan-Out/Fan-In** | 12 | 16 | 0.6 ms | 6.3 ms | 4.8 KB |
| **Binary Tree** | 31 | 30 | 1.1 ms | 13.1 ms | 11.9 KB |
| **Deep Chain** | 50 | 49 | 1.9 ms | 3.2 ms | 20.2 KB |
| **Diamond Lattice** | 64 | 112 | 2.8 ms | 45.3 ms | 26.8 KB |

## Bundle size (WASM, `opt-level = "z"` + LTO + `wasm-opt -Oz`)

- Arena mode (no-alloc): **~39 KB** (17 KB gzipped)
- Full mode (`std` + `generic`): **~93 KB** (39 KB gzipped)

Re-measured at 0.9.1 with the harness described in the 0.10 section
above, these come out at 46.4 KB and 95.3 KB (20.2 / 40.0 KB
gzipped) — close enough to treat the figures above as sound.
