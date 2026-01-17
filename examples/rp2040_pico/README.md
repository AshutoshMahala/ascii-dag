# ascii-dag on RP2040 / Raspberry Pi Pico

This example demonstrates running `ascii-dag` on embedded hardware (ARM Cortex-M0+), rendering DAGs and sending output over USB serial.

## Requirements

1. **Rust toolchain for ARM Cortex-M0+**:
   ```bash
   rustup target add thumbv6m-none-eabi
   ```

2. **probe-rs** (optional, for easy flashing):
   ```bash
   cargo install probe-rs-tools
   ```

3. **Raspberry Pi Pico** connected via USB

## Building

```bash
cd examples/rp2040_pico
cargo build --release
```

The binary will be at `target/thumbv6m-none-eabi/release/ascii-dag-pico`.

## Flashing

### Option A: UF2 (No debugger needed)

1. Convert ELF to UF2:
   ```bash
   cargo install elf2uf2-rs
   elf2uf2-rs target/thumbv6m-none-eabi/release/ascii-dag-pico ascii-dag-pico.uf2
   ```

2. Hold BOOTSEL button on Pico while plugging in USB
3. Copy `ascii-dag-pico.uf2` to the `RPI-RP2` drive that appears

### Option B: probe-rs (With debugger or Pico as debug probe)

```bash
cargo run --release
```

## Viewing Output

Connect to the USB serial port:

**Linux/macOS:**
```bash
screen /dev/ttyACM0 115200
# or
minicom -D /dev/ttyACM0 -b 115200
```

**Windows:**
- Open Device Manager, find the COM port (e.g., COM3)
- Use PuTTY: Serial, COM3, 115200 baud

## Expected Output

```
================================
  ascii-dag on RP2040 Pico!
================================

1. Error Chain:
[ParseError] → [ValidationError] → [IOError]

2. Diamond Pattern:
    [Root]
       │
    ┌──┴──┐
    ↓     ↓
 [Left] [Right]
    │     │
    └──┬──┘
       ↓
    [Merge]

3. Build Pipeline:
[Fetch] → [Compile] → [Test] → [Package]

4. Multi-Convergence:
[Src1]  [Src2]  [Src3]
   │      │       │
   └──────┴───────┘
          ↓
       [Final]

--------------------------------
Heap: ~2048 bytes used, ~30720 bytes free
--------------------------------
Done! ascii-dag works on RP2040.
```

## Memory Usage

- **Flash**: ~50-60 KB (including ascii-dag)
- **RAM**: ~32 KB heap allocated, ~2-4 KB actually used for small DAGs
- **Stack**: ~4 KB

The RP2040 has 264 KB RAM, so there's plenty of headroom.

## Troubleshooting

1. **No USB serial port appears**: 
   - Make sure you're not in BOOTSEL mode
   - Try a different USB cable (some are charge-only)

2. **Build fails with linker errors**:
   - Ensure `thumbv6m-none-eabi` target is installed
   - Check that `.cargo/config.toml` has the correct target

3. **Garbled output**:
   - Ensure terminal is set to 115200 baud
   - Check that your terminal supports Unicode (UTF-8)
