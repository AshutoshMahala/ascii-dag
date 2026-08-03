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

The 0.10 render engine adds roughly 4–4.5% for engine users, and the
rank directions the amount measured above.
