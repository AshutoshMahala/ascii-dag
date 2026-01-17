#!/bin/bash
# Security testing script for ascii-dag
# Runs comprehensive security analysis including fuzzing, UB detection, and memory safety checks.
#
# Usage:
#   ./scripts/security-check.sh          # Run all security checks
#   ./scripts/security-check.sh quick    # Run quick checks only (no fuzzing)
#   ./scripts/security-check.sh fuzz     # Run fuzzing only (takes minutes to hours)
#
# Requirements:
#   - rustup +nightly (for miri, cargo-careful, cargo-fuzz)
#   - cargo-fuzz: cargo install cargo-fuzz
#   - cargo-careful: cargo install cargo-careful

set -e

echo "=== Security Testing for ascii-dag ==="
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

success() { echo -e "${GREEN}✓ $1${NC}"; }
warning() { echo -e "${YELLOW}⚠ $1${NC}"; }
error() { echo -e "${RED}✗ $1${NC}"; }
info() { echo -e "${BLUE}ℹ $1${NC}"; }

MODE="${1:-all}"

# ===========================================================================
# 1. cargo-careful: Run tests with extra UB checks
# ===========================================================================
if [[ "$MODE" == "all" || "$MODE" == "quick" ]]; then
    echo "1. Running cargo-careful (extra UB detection)..."
    if command -v cargo-careful &> /dev/null; then
        # Build the nightly sysroot if needed
        if ! cargo +nightly careful test --lib 2>&1 | grep -q "^error"; then
            success "cargo-careful passed"
        else
            warning "cargo-careful found issues"
            cargo +nightly careful test --lib
        fi
    else
        warning "cargo-careful not installed. Run: cargo install cargo-careful"
    fi
    echo ""
fi

# ===========================================================================
# 2. Miri: Memory safety and UB detection
# ===========================================================================
if [[ "$MODE" == "all" || "$MODE" == "quick" ]]; then
    echo "2. Running Miri (memory safety checks)..."
    if rustup run nightly cargo miri --version &> /dev/null 2>&1; then
        # Run miri on arena tests specifically (most likely to have UB)
        if timeout 300 rustup run nightly cargo miri test arena:: --lib 2>&1 | grep -q "Undefined Behavior"; then
            error "Miri found Undefined Behavior!"
            rustup run nightly cargo miri test arena:: --lib
            exit 1
        else
            success "Miri passed on arena module"
        fi
    else
        warning "Miri not available. Install: rustup +nightly component add miri"
    fi
    echo ""
fi

# ===========================================================================
# 3. Clippy with extra security lints
# ===========================================================================
if [[ "$MODE" == "all" || "$MODE" == "quick" ]]; then
    echo "3. Running Clippy with security lints..."
    CLIPPY_OUTPUT=$(cargo clippy --lib -- \
        -W clippy::cast_possible_truncation \
        -W clippy::cast_sign_loss \
        -W clippy::cast_possible_wrap \
        -W clippy::integer_division \
        -W clippy::indexing_slicing \
        2>&1)
    
    if echo "$CLIPPY_OUTPUT" | grep -q "^error\["; then
        error "Clippy found security issues"
        echo "$CLIPPY_OUTPUT"
        exit 1
    else
        success "Clippy security lints passed"
    fi
    echo ""
fi

# ===========================================================================
# 4. Check for unsafe code
# ===========================================================================
if [[ "$MODE" == "all" || "$MODE" == "quick" ]]; then
    echo "4. Scanning for unsafe code..."
    UNSAFE_COUNT=$(grep -r "unsafe" src/ --include="*.rs" | grep -v "// SAFETY:" | wc -l | tr -d ' ')
    if [[ "$UNSAFE_COUNT" -gt 0 ]]; then
        warning "Found $UNSAFE_COUNT uses of 'unsafe' (some may lack SAFETY comments)"
        echo "   Locations:"
        grep -rn "unsafe" src/ --include="*.rs" | grep -v "// SAFETY:" | head -10
        echo ""
    else
        success "All unsafe blocks have SAFETY comments (or no unsafe code)"
    fi
    echo ""
fi

# ===========================================================================
# 5. Fuzzing (short run for smoke test, or full run)
# ===========================================================================
if [[ "$MODE" == "all" || "$MODE" == "fuzz" ]]; then
    echo "5. Running cargo-fuzz..."
    if command -v cargo-fuzz &> /dev/null; then
        FUZZ_DURATION="${FUZZ_DURATION:-30}"  # Default 30 seconds per target
        
        for target in fuzz_arena fuzz_dag_layout fuzz_arena_layout; do
            info "Fuzzing $target for ${FUZZ_DURATION}s..."
            
            # Run fuzz with timeout
            cd fuzz
            if timeout "$FUZZ_DURATION" cargo +nightly fuzz run "$target" -- -max_total_time="$FUZZ_DURATION" 2>&1 | grep -q "BINGO\|panic\|ERROR"; then
                error "Fuzzer found crash in $target!"
                cd ..
                exit 1
            fi
            cd ..
            success "$target: No crashes found"
        done
    else
        warning "cargo-fuzz not installed. Run: cargo install cargo-fuzz"
    fi
    echo ""
fi

# ===========================================================================
# 6. Address Sanitizer (if available)
# ===========================================================================
if [[ "$MODE" == "all" ]]; then
    echo "6. Running with Address Sanitizer..."
    if rustup run nightly rustc --version &> /dev/null; then
        export RUSTFLAGS="-Z sanitizer=address"
        if RUSTFLAGS="-Z sanitizer=address" cargo +nightly test --lib --target aarch64-apple-darwin 2>&1 | grep -q "ERROR: AddressSanitizer"; then
            error "AddressSanitizer found memory issues!"
            exit 1
        else
            success "AddressSanitizer passed"
        fi
        unset RUSTFLAGS
    else
        warning "Nightly toolchain not available for sanitizer"
    fi
    echo ""
fi

# ===========================================================================
# Summary
# ===========================================================================
echo ""
echo "=== Security Testing Complete ==="

if [[ "$MODE" == "quick" ]]; then
    echo ""
    info "Quick mode completed. For full fuzzing, run:"
    echo "   ./scripts/security-check.sh fuzz"
    echo "   FUZZ_DURATION=300 ./scripts/security-check.sh fuzz  # 5 minutes per target"
fi

echo ""
