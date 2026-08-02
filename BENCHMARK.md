# Benchmarks

> **Status note:** the tables below were measured on **0.9.x**. The
> 0.10 unified render engine changes several of them substantially —
> e.g. Wide-Fan 50k *render* went from ~17 s to ~0.08 s (merged-run
> painting), and Massive-Diamond 50k *CSR layout* from ~6.2 s to
> ~88 ms (O(E log E) two-node-cycle detection). A full re-measurement
> lands with the 0.10 release.

## How to run

```bash
cargo run --release --example benchmark --features arena
cargo run --release --example stress_test --features arena
```

Embedded numbers come from physical hardware:
`examples/rp2040_pico` (Raspberry Pi Pico) and `examples/esp32s3`
(Seeed XIAO ESP32-S3). Desktop numbers: Apple M2 Ultra, release build.
"Heap" is the default `Graph` pipeline; "Arena" is the
CSR/no-alloc pipeline (`--features arena`).

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

*Chain = linear (best case), Diamond = skip-level edges (stress),
WideFan = fan-out/fan-in (crossing worst case).*

## Embedded (RP2040 / Cortex-M0+ @ 125 MHz)

| Graph | Nodes | Mode | Build | Compute | Render | **Total** | RAM | Speedup |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Chain 10** | 10 | Heap | 0.6 ms | 3.1 ms | 0.6 ms | **4.3 ms** | 5.0 KB | |
| | | Arena | 0.3 ms | 1.1 ms | 0.5 ms | **1.9 ms** | **1.9 KB** | **2.3x** |
| **Chain 50** | 50 | Heap | 2.5 ms | 13.2 ms | 2.4 ms | **18.2 ms** | 23.6 KB | |
| | | Arena | 0.6 ms | 2.9 ms | 2.7 ms | **6.2 ms** | **9.1 KB** | **3.0x** |
| **Chain 100** | 100 | Heap | 5.1 ms | 28.0 ms | 4.9 ms | **38.0 ms** | 47.1 KB | |
| | | Arena | 1.4 ms | 6.2 ms | 7.5 ms | **15.0 ms** | **18.2 KB** | **2.5x** |

Longan Nano (RISC-V GD32VF103, 20 KB RAM) runs the no-alloc arena
mode on its 160×80 LCD: see `assets/longan-nano.png`.

## Embedded (ESP32-S3 / Xtensa LX7 @ 240 MHz)

| Graph | Nodes | Edges | Build | Render | RAM |
| :--- | ---: | ---: | ---: | ---: | ---: |
| **Diamond** | 4 | 4 | 0.5ms | 3.7ms | 1.5 KB |
| **Build Pipeline** | 10 | 12 | 0.5ms | 7.7ms | 3.6 KB |
| **Fan-Out/Fan-In** | 12 | 16 | 0.6ms | 6.3ms | 4.8 KB |
| **Binary Tree** | 31 | 30 | 1.1ms | 13.1ms | 11.9 KB |
| **Deep Chain** | 50 | 49 | 1.9ms | 3.2ms | 20.2 KB |
| **Diamond Lattice** | 64 | 112 | 2.8ms | 45.3ms | 26.8 KB |

## Scalability (stress shapes, 0.9.x — see status note)

| Topology | Nodes | Mode | Time | Output Size | Speedup |
| :--- | ---: | :--- | ---: | ---: | ---: |
| **Diamond** | 20,164 | Heap | 450ms | 0.61 MB | |
| | | Arena | 994ms | | 0.5x |
| **Diamond** | 50,176 | Heap | 2.4s | 1.52 MB | |
| | | Arena | 6.1s | | 0.4x |
| **Wide Fan** | 50,000 | Heap | 18.0s | 5.82 MB | |
| | | Arena | 4.6s | | **3.9x** |

Known 0.10 improvements to these shapes (measured during the engine
rewrite): Wide-Fan 50k render 17 s → 0.08 s (both pipelines); Massive
Diamond 50k arena layout+render 6.2 s → 88 ms; plan-only
time-to-first-byte ~15 ms on Wide-Fan 50k.

## Node-content storage overhead (0.10, measured)

`cargo run --release --example content_overhead --features arena` —
10,000-node chain, per-kind, on the public arena-estimate contract
(what embedded users provision against):

| Kind | Build | Layout+render | CSR estimate delta | Layout estimate delta |
|---|---:|---:|---:|---:|
| all simple | 2.6 ms | 32.5 ms | baseline | baseline |
| all boxed | 2.6 ms | 33.3 ms | **+0.0 B/node** | **+0.0 B/node** |
| all custom (painter + 8 B payload) | 2.5 ms | 28.6 ms | **+40.0 B/node** | **+40.0 B/node** |

This demonstrates the sparse+packed storage design (NC-N1): built-in
kinds cost nothing (the tag packs into existing per-node flags), and
only nodes declaring a painter/payload pay — 32 B per entry plus their
payload bytes. Render time is unaffected.

## Rank-direction support: what LR/RL cost (0.10, measured)

The four directions share **one** layout pipeline, parameterized by a
zero-sized axis profile the compiler monomorphizes away. Both profiles
(`Vertical` for TB/BT, `Horizontal` for LR/RL) are reachable from
`compute_layout()`, so the binary carries two stamped copies of the
layout stage.

Measured against the pre-direction-block baseline, release build,
stripped, Apple M2 Ultra:

| Binary | Before | After | Delta |
|---|---:|---:|---:|
| `hero` example | 502,552 B | 585,976 B | **+16.6%** |
| `benchmark` example | 519,048 B | 585,800 B | **+12.9%** |

Layout time, min-of-3 interleaved runs (the two profiles execute the
same algorithm — TD/BT output is byte-identical before and after — so
any delta is scheduling plus instruction-cache pressure from the
larger binary, not extra work):

| Shape | Heap | Arena |
|---|---:|---:|
| Chain 100 / 250 | +9% / +7% | +6% / +20% |
| Diamond 100 / 200 | +2% / +2% | +3% / +3% |
| WideFan 100 / 500 | −1% / −2% | −2% / −11% |

Net: a wash on time, ~13–17% on size. If the size matters more than
the feature for a given target, the fallback D1 recorded is to trade
the second monomorphization for an enum branch in the cold parts of
the pipeline.

## Bundle size (WASM, `opt-level = "z"` + LTO + `wasm-opt -Oz`)

- Arena mode (no-alloc): **~39 KB** (17 KB gzipped)
- Full mode (`std` + `generic`): **~93 KB** (39 KB gzipped)
- 0.10 engine adds ~4–4.5% for engine users; legacy-API users are
  unaffected (the linker strips it).
