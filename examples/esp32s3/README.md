# ascii-dag on ESP32-S3

Performance benchmark demonstrating `ascii-dag` running on ESP32-S3 (Xtensa LX7 @ 240 MHz).

## Requirements

### Hardware

- ESP32-S3 development board (e.g., ESP32-S3-DevKitC)

### Software

### 1. Install ESP Rust Toolchain

> **Note:** The ESP toolchain is a fork of Rust nightly (currently based on 1.85/1.86).
> This is separate from your system Rust installation and only used for Xtensa targets.
> The main `ascii-dag` crate works with standard Rust 1.92+.

```bash
# Install espup
cargo install espup

# Install ESP Rust toolchain
espup install

# Source the environment (run this in each new terminal)
# Linux/macOS:
. $HOME/export-esp.sh
# Windows PowerShell:
. $HOME\export-esp.ps1
```

### 2. Install espflash

```bash
cargo install espflash
```

## Building

```bash
cd examples/esp32s3
cargo build --release
```

## Flashing & Monitoring

```bash
cargo run --release
```

Or manually:

```bash
espflash flash target/xtensa-esp32s3-none-elf/release/esp32s3-ascii-dag --monitor
```

## Expected Output

```
╔══════════════════════════════════════════════════╗
║     ascii-dag Performance Test on ESP32-S3      ║
║          Xtensa LX7 @ 240 MHz, 512KB RAM        ║
╚══════════════════════════════════════════════════╝

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
BENCHMARK 1: Diamond Pattern (4 nodes, 4 edges)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    [Root]
       │
    ┌──┴──┐
    ↓     ↓
 [Left] [Right]
    │     │
    └──┬──┘
       ↓
    [Merge]

Build:     ~80 µs
Render:   ~250 µs
Heap:    ~1500 bytes
Output:   ~200 bytes

...
```

## Features

- **4 Benchmark scenarios**: Diamond, Pipeline, Fan-out/Fan-in, Binary tree
- **Microsecond timing**: Using ESP32-S3 hardware timer
- **Heap tracking**: Real-time memory usage monitoring
- **Fast performance**: Xtensa LX7 dual-core @ 240 MHz

## Memory Usage

- **Heap**: 32 KB allocated
- **Small DAGs**: ~1-2 KB
- **Large DAGs (31 nodes)**: ~4-6 KB
- **ESP32-S3**: 512 KB SRAM total (plenty of headroom!)

## Troubleshooting

### espflash not found
```bash
cargo install espflash
```

### Toolchain issues
```bash
espup update
. $HOME/export-esp.sh  # or export-esp.ps1 on Windows
```

### Device not detected
- Ensure USB cable supports data (not just charging)
- Check device appears in Device Manager (Windows) or `ls /dev/tty*` (Linux/macOS)
- Try different USB port
- Install CH340 driver if using cheap USB-Serial adapter

### Build fails
```bash
# Clean and rebuild
cargo clean
cargo build --release
```
