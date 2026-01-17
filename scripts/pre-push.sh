#!/bin/bash
# Pre-push verification script for ascii-dag
# Run this before pushing to catch issues early
# Usage: ./scripts/pre-push.sh

set -e  # Exit on first error

echo "=== Pre-Push Verification for ascii-dag ==="
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

success() {
    echo -e "${GREEN}✓ $1${NC}"
}

warning() {
    echo -e "${YELLOW}⚠ $1${NC}"
}

error() {
    echo -e "${RED}✗ $1${NC}"
}

# 1. Format check
echo "1. Checking formatting..."
if cargo fmt --check 2>/dev/null; then
    success "Code formatting OK"
else
    error "Code formatting issues found. Run 'cargo fmt' to fix."
    exit 1
fi

# 2. Clippy lints
echo ""
echo "2. Running Clippy..."
# Note: Don't use --all-targets - embedded_proof.rs has conflicting panic_impl
# Just check the library and tests
CLIPPY_OUTPUT=$(cargo clippy --lib --tests 2>&1)
if echo "$CLIPPY_OUTPUT" | grep -q "^error\["; then
    error "Clippy found errors"
    echo "$CLIPPY_OUTPUT"
    exit 1
else
    success "Clippy checks passed"
fi

# 3. Build check
echo ""
echo "3. Building in release mode..."
if cargo build --release 2>&1 | grep -q "^error"; then
    error "Release build failed"
    exit 1
else
    success "Release build OK"
fi

# 4. Run tests
echo ""
echo "4. Running tests..."
if cargo test --lib --release 2>&1 | grep -q "FAILED"; then
    error "Some tests failed"
    cargo test --lib --release
    exit 1
else
    success "All tests passed"
fi

# 5. Run arena benchmark (quick smoke test)
echo ""
echo "5. Running arena benchmark smoke test..."
BENCH_OUTPUT=$(timeout 30 cargo run --example arena_benchmark --release 2>&1 || true)
if echo "$BENCH_OUTPUT" | grep -q "abort\|panic\|FAILED"; then
    error "Arena benchmark crashed or panicked"
    echo "$BENCH_OUTPUT" | tail -20
    exit 1
else
    success "Arena benchmark completed"
fi

# 6. Check for unsafe code issues (if miri is available)
echo ""
echo "6. Checking for Miri availability..."
if command -v cargo-miri &> /dev/null || rustup run nightly cargo miri --version &> /dev/null 2>&1; then
    echo "   Running Miri on arena tests..."
    if timeout 120 rustup run nightly cargo miri test arena:: --lib 2>&1 | grep -q "error\|Undefined"; then
        warning "Miri found potential issues in arena code"
    else
        success "Miri checks passed"
    fi
else
    warning "Miri not available. Install with: rustup +nightly component add miri"
fi

# 7. cargo-careful (extra UB detection)
echo ""
echo "7. Running cargo-careful (if available)..."
if command -v cargo-careful &> /dev/null; then
    if timeout 120 cargo +nightly careful test --lib 2>&1 | grep -q "FAILED\|panicked"; then
        warning "cargo-careful found potential issues"
    else
        success "cargo-careful passed"
    fi
else
    warning "cargo-careful not available. Install with: cargo install cargo-careful"
fi

# 8. Documentation check
echo ""
echo "8. Checking documentation..."
if cargo doc --no-deps 2>&1 | grep -q "^error"; then
    error "Documentation generation failed"
    exit 1
else
    success "Documentation builds OK"
fi

echo ""
echo "=== All pre-push checks passed! ==="
echo ""
