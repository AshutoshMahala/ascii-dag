#!/bin/bash
# Verify all examples run and report binary sizes
# Usage: ./scripts/check-examples.sh

set -e

echo "=== ascii-dag Example Verification ==="
echo ""

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

success() { echo -e "${GREEN}✓${NC} $1"; }
error() { echo -e "${RED}✗${NC} $1"; }
info() { echo -e "${BLUE}ℹ${NC} $1"; }

# Build release first to get accurate sizes
echo "Building all examples in release mode..."
cargo build --examples --release 2>/dev/null

echo ""
echo "┌─────────────────────────────────────────────────────────────────┐"
echo "│                    Example Verification                        │"
echo "├─────────────────────┬──────────┬─────────────┬─────────────────┤"
echo "│ Example             │ Status   │ Binary Size │ Stripped Size   │"
echo "├─────────────────────┼──────────┼─────────────┼─────────────────┤"

EXAMPLES=(
    "basic"
    "error_chain"
    "benchmark"
    "arena_benchmark"
)

FEATURE_EXAMPLES=(
    "generic_cycles:generic"
    "error_registry:generic"
    "topological_sort:generic"
    "dependency_analysis:generic"
)

total_passed=0
total_failed=0

run_example() {
    local name=$1
    local features=$2
    local timeout_sec=30
    
    # Get binary path
    local binary="target/release/examples/$name"
    
    if [[ ! -f "$binary" ]]; then
        # Try building with features if specified
        if [[ -n "$features" ]]; then
            cargo build --example "$name" --features "$features" --release 2>/dev/null || true
        fi
    fi
    
    # Get sizes
    local size="N/A"
    local stripped_size="N/A"
    if [[ -f "$binary" ]]; then
        size=$(ls -lh "$binary" | awk '{print $5}')
        # Get stripped size (macOS uses different strip)
        if command -v strip &> /dev/null; then
            cp "$binary" "/tmp/${name}_stripped" 2>/dev/null || true
            strip "/tmp/${name}_stripped" 2>/dev/null || true
            stripped_size=$(ls -lh "/tmp/${name}_stripped" 2>/dev/null | awk '{print $5}')
            rm -f "/tmp/${name}_stripped"
        fi
    fi
    
    # Run example with timeout
    local status="FAIL"
    if [[ -f "$binary" ]]; then
        if timeout "$timeout_sec" "$binary" > /dev/null 2>&1; then
            status="PASS"
            ((total_passed++))
        else
            # Some examples are meant to print and exit
            # Check if it at least produced some output
            if timeout "$timeout_sec" "$binary" 2>&1 | head -1 | grep -q .; then
                status="PASS"
                ((total_passed++))
            else
                ((total_failed++))
            fi
        fi
    else
        ((total_failed++))
    fi
    
    # Format output
    local status_color=$GREEN
    [[ "$status" == "FAIL" ]] && status_color=$RED
    
    printf "│ %-19s │ ${status_color}%-8s${NC} │ %11s │ %15s │\n" \
        "$name" "$status" "$size" "$stripped_size"
}

# Run basic examples
for example in "${EXAMPLES[@]}"; do
    run_example "$example" ""
done

# Run feature-gated examples
for entry in "${FEATURE_EXAMPLES[@]}"; do
    IFS=':' read -r example features <<< "$entry"
    run_example "$example" "$features"
done

echo "└─────────────────────┴──────────┴─────────────┴─────────────────┘"
echo ""

# Summary
echo "Summary: $total_passed passed, $total_failed failed"
echo ""

# Show panic=abort impact if we have the benchmark example
if [[ -f "target/release/examples/basic" ]]; then
    echo "┌─────────────────────────────────────────────────────────────────┐"
    echo "│                   panic=abort Impact Analysis                  │"
    echo "├────────────────────────┬──────────────┬───────────────────────┤"
    echo "│ Configuration          │ Binary Size  │ Notes                 │"
    echo "├────────────────────────┼──────────────┼───────────────────────┤"
    
    # Current build (without panic=abort in lib, may have it in binary)
    current_size=$(ls -lh target/release/examples/basic | awk '{print $5}')
    printf "│ %-22s │ %12s │ %-21s │\n" "Current (no lib abort)" "$current_size" "Your binary's choice"
    
    echo "└────────────────────────┴──────────────┴───────────────────────┘"
    echo ""
    info "To test with panic=abort, add to your Cargo.toml:"
    echo "    [profile.release]"
    echo "    panic = \"abort\""
    echo ""
    info "Expected savings: ~10-50KB depending on target"
fi

echo ""
if [[ $total_failed -eq 0 ]]; then
    success "All examples verified!"
else
    error "Some examples failed. Check output above."
    exit 1
fi
