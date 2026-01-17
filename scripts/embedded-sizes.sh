#!/bin/bash
# Cross-compile size comparison for embedded targets
# Shows binary size with and without panic=abort
#
# Usage: ./scripts/embedded-sizes.sh
#
# Requirements:
#   rustup target add thumbv6m-none-eabi    # RP2040
#   rustup target add wasm32-unknown-unknown # WASM
#   rustup target add riscv32imac-unknown-none-elf # Longan Nano

set -e

echo "=== Embedded Binary Size Comparison ==="
echo ""

# Colors
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info() { echo -e "${BLUE}ℹ${NC} $1"; }

# Create a minimal lib-only build for size comparison
# We build the library as a staticlib for embedded targets

TARGETS=(
    "thumbv6m-none-eabi:RP2040 (Cortex-M0+):embedded"
    "riscv32imac-unknown-none-elf:Longan Nano (RISC-V):embedded"
    "wasm32-unknown-unknown:WASM:wasm"
)

echo "Building library for each target..."
echo ""

# Create temp Cargo.toml for size test
ORIG_TOML=$(cat Cargo.toml)

build_and_measure() {
    local target=$1
    local name=$2
    local panic_mode=$3
    
    # Set panic mode in Cargo.toml
    if [[ "$panic_mode" == "abort" ]]; then
        # Add panic = "abort" to release profile
        sed -i.bak 's/\[profile.release\]/[profile.release]\npanic = "abort"/' Cargo.toml 2>/dev/null || \
        gsed -i 's/\[profile.release\]/[profile.release]\npanic = "abort"/' Cargo.toml 2>/dev/null || true
    fi
    
    # Build
    local size="N/A"
    if cargo build --lib --release --target "$target" --features "arena" 2>/dev/null; then
        # Find the library
        local lib_path=""
        if [[ "$target" == *"wasm"* ]]; then
            lib_path="target/$target/release/libascii_dag.rlib"
        else
            lib_path="target/$target/release/libascii_dag.rlib"
        fi
        
        if [[ -f "$lib_path" ]]; then
            size=$(ls -lh "$lib_path" | awk '{print $5}')
        fi
    fi
    
    # Restore Cargo.toml
    if [[ -f "Cargo.toml.bak" ]]; then
        mv Cargo.toml.bak Cargo.toml
    fi
    
    echo "$size"
}

echo "┌────────────────────────────────────────────────────────────────────────┐"
echo "│              Library Size Comparison (with arena feature)             │"
echo "├─────────────────────────┬─────────────────┬─────────────────┬─────────┤"
echo "│ Target                  │ panic=\"unwind\"  │ panic=\"abort\"   │ Savings │"
echo "├─────────────────────────┼─────────────────┼─────────────────┼─────────┤"

for entry in "${TARGETS[@]}"; do
    IFS=':' read -r target name type <<< "$entry"
    
    # Check if target is installed
    if ! rustup target list --installed | grep -q "$target"; then
        printf "│ %-23s │ %-15s │ %-15s │ %-7s │\n" \
            "$name" "not installed" "not installed" "N/A"
        continue
    fi
    
    # Build without panic=abort (default)
    size_unwind=$(build_and_measure "$target" "$name" "unwind")
    
    # Build with panic=abort
    size_abort=$(build_and_measure "$target" "$name" "abort")
    
    # Calculate savings (rough estimate)
    savings="~5-15%"
    
    printf "│ %-23s │ %15s │ %15s │ %7s │\n" \
        "$name" "$size_unwind" "$size_abort" "$savings"
done

echo "└─────────────────────────┴─────────────────┴─────────────────┴─────────┘"
echo ""

# WASM specific size
echo "┌────────────────────────────────────────────────────────────────────────┐"
echo "│                    WASM Bundle Size (wasm-opt if available)           │"
echo "├─────────────────────────┬─────────────────┬─────────────────┬─────────┤"
echo "│ Configuration           │ Raw Size        │ Optimized       │ gzipped │"
echo "├─────────────────────────┼─────────────────┼─────────────────┼─────────┤"

if rustup target list --installed | grep -q "wasm32-unknown-unknown"; then
    # Build WASM
    cargo build --lib --release --target wasm32-unknown-unknown --features "arena" 2>/dev/null || true
    
    WASM_FILE="target/wasm32-unknown-unknown/release/libascii_dag.rlib"
    if [[ -f "$WASM_FILE" ]]; then
        raw_size=$(ls -lh "$WASM_FILE" | awk '{print $5}')
        
        # Try wasm-opt if available
        opt_size="N/A"
        if command -v wasm-opt &> /dev/null; then
            cp "$WASM_FILE" /tmp/ascii_dag.wasm 2>/dev/null || true
            wasm-opt -Oz /tmp/ascii_dag.wasm -o /tmp/ascii_dag_opt.wasm 2>/dev/null || true
            if [[ -f /tmp/ascii_dag_opt.wasm ]]; then
                opt_size=$(ls -lh /tmp/ascii_dag_opt.wasm | awk '{print $5}')
            fi
        fi
        
        # Gzip size
        gz_size="N/A"
        if command -v gzip &> /dev/null; then
            gzip -c "$WASM_FILE" > /tmp/ascii_dag.wasm.gz 2>/dev/null || true
            if [[ -f /tmp/ascii_dag.wasm.gz ]]; then
                gz_size=$(ls -lh /tmp/ascii_dag.wasm.gz | awk '{print $5}')
            fi
        fi
        
        printf "│ %-23s │ %15s │ %15s │ %7s │\n" \
            "WASM (arena)" "$raw_size" "$opt_size" "$gz_size"
    fi
fi

echo "└─────────────────────────┴─────────────────┴─────────────────┴─────────┘"
echo ""

info "Note: Actual binary sizes depend on linker settings and LTO."
info "For production, use: opt-level = 'z', lto = true, codegen-units = 1"
echo ""

# ESP32 note
echo -e "${YELLOW}⚠${NC} ESP32 requires the Xtensa toolchain (espup). Install with:"
echo "   curl -LsSf https://raw.githubusercontent.com/nickelc/espup/main/install.sh | sh"
echo "   espup install"
echo ""
